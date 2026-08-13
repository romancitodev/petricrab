use std::collections::HashSet;

use eframe::egui;

use crate::app::{NodeId, PetriApp};
use crate::editor::{card, center_on_node, marking_chips, section_label};
use crate::icons;
use crate::model::fire::enabled_transitions;
use crate::model::{
  Marking as ModelMarking, PetriNet, PlaceId as ModelPlaceId, TransitionId as ModelTransitionId,
};
use crate::theme;

/// Drives the real canvas through a fixed firing sequence (a witness path: a deadlock, a
/// liveness example, a route to a home state or a reachability-graph node): every frame,
/// overwrites the live net's marking with the recorded state for the current step, so the
/// canvas (real positions, real arcs, real colors) shows tokens moving along the route. Fires
/// only the one transition that's next in the recorded path, no free choice — `editor::canvas`
/// disables its own editing/selection while this is open, but pan/zoom stay live.
pub struct RouteModal {
  states: Vec<ModelMarking>,
  transitions: Vec<ModelTransitionId>,
  step: usize,
  original_marking: ModelMarking,
  route_places: HashSet<ModelPlaceId>,
  route_transitions: HashSet<ModelTransitionId>,
}

impl RouteModal {
  /// `states[0]` is where the route starts; `states[i + 1]` is `states[i]` after firing
  /// `transitions[i]`. `states` can be shorter than `transitions.len() + 1` for a witness that
  /// came from a coverability graph (see `analysis::replay_path`'s doc).
  pub fn new(net: &PetriNet, states: Vec<ModelMarking>, transitions: Vec<ModelTransitionId>) -> Self {
    let route_places = states
      .iter()
      .flat_map(|m| m.iter().filter(|&(_, &tokens)| tokens > 0).map(|(&p, _)| p))
      .collect();
    let route_transitions = transitions.iter().copied().collect();

    Self {
      states,
      transitions,
      step: 0,
      original_marking: net.marking(),
      route_places,
      route_transitions,
    }
  }

  /// Every place that ever holds a token at some step of this route — `editor::draw_net` dims
  /// everything else while this modal is open.
  pub fn route_places(&self) -> &HashSet<ModelPlaceId> {
    &self.route_places
  }

  /// Every transition fired along this route.
  pub fn route_transitions(&self) -> &HashSet<ModelTransitionId> {
    &self.route_transitions
  }

  /// The transition about to fire at the current step — `editor::draw_net` rings it so it stands
  /// out from the rest of the (already brighter-than-everything-else) route.
  pub fn current_transition(&self) -> Option<ModelTransitionId> {
    self.transitions.get(self.step).copied()
  }

  /// Every transition already fired on the way to the current step.
  pub fn visited_transitions(&self) -> HashSet<ModelTransitionId> {
    self.transitions[..self.step].iter().copied().collect()
  }

  /// The transition to point the camera at: the one about to fire, or — once the route has run
  /// out (reached the deadlock) — the last one that did.
  fn focus_transition(&self) -> Option<ModelTransitionId> {
    self
      .current_transition()
      .or_else(|| self.step.checked_sub(1).and_then(|i| self.transitions.get(i).copied()))
  }

  fn step_back(&mut self) {
    self.step = self.step.saturating_sub(1);
  }

  fn step_forward(&mut self) {
    if self.step + 1 < self.states.len() {
      self.step += 1;
    }
  }

  /// Draws the floating step controls and drives the canvas for this frame. Returns `false`
  /// once it should close — the caller must then call `close` to restore the net's marking.
  pub fn show(&mut self, ctx: &egui::Context, app: &mut PetriApp) -> bool {
    app.net.set_marking(&self.states[self.step]);

    let step_before = self.step;
    let mut keep_open = true;

    ctx.input(|i| {
      if i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::Space) {
        self.step_forward();
      }
      if i.key_pressed(egui::Key::ArrowLeft) {
        self.step_back();
      }
      if i.key_pressed(egui::Key::Escape) {
        keep_open = false;
      }
    });

    egui::Area::new(egui::Id::new("route-modal-popup"))
      .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -72.0))
      .show(ctx, |ui| {
        let visuals = ui.visuals().clone();
        egui::Frame::default()
          .fill(visuals.panel_fill)
          .stroke(egui::Stroke::new(1.0, visuals.window_stroke.color))
          .corner_radius(theme::RADIUS_LG)
          .shadow(visuals.window_shadow)
          .inner_margin(egui::Margin::symmetric(14, 10))
          .show(ui, |ui| {
            ui.set_width(300.0);
            ui.horizontal(|ui| {
              ui.label(icons::icon("route", 14.0));
              ui.strong("Ruta");
              ui.weak(format!("paso {}/{}", self.step, self.states.len() - 1));
            });
            ui.add_space(10.0);

            section_label(ui, "Marking");
            ui.add_space(6.0);
            card(ui, |ui| {
              marking_chips(ui, &app.net, &self.states[self.step]);
            });
            ui.add_space(10.0);

            match self.current_transition() {
              Some(t) => {
                let btn = egui::Button::new((
                  icons::icon("zap", 13.0),
                  format!("Disparar {}", app.net.transition_label(t)),
                ))
                .corner_radius(6.0);
                if ui.add_sized([ui.available_width(), 30.0], btn).clicked() {
                  self.step_forward();
                }
              }
              None if self.step + 1 < self.states.len() => {
                ui.horizontal_wrapped(|ui| {
                  ui.label(icons::icon("triangle-alert", 14.0).color(ui.visuals().warn_fg_color));
                  ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "No se pudo reproducir el resto de la ruta en la red actual.",
                  );
                });
              }
              None if enabled_transitions(&app.net, &self.states[self.step]).is_empty() => {
                ui.horizontal_wrapped(|ui| {
                  ui.label(icons::icon("ban", 14.0).color(theme::danger()));
                  ui.colored_label(theme::danger(), "Deadlock: sin transiciones habilitadas");
                });
              }
              None => {
                ui.horizontal_wrapped(|ui| {
                  ui.label(icons::icon("flag", 14.0).color(theme::success()));
                  ui.colored_label(theme::success(), "Fin de la ruta");
                });
              }
            }
            ui.add_space(10.0);

            ui.horizontal(|ui| {
              if ui
                .add_enabled(self.step > 0, egui::Button::new("Atrás"))
                .clicked()
              {
                self.step_back();
              }
              if ui.button("Reiniciar").clicked() {
                self.step = 0;
              }
              if ui.button("Cerrar").clicked() {
                keep_open = false;
              }
            });
            ui.add_space(4.0);
            ui.weak("← → / space para avanzar, esc para cerrar");
          });
      });

    if self.step != step_before {
      if let Some(t) = self.focus_transition() {
        center_on_node(app, NodeId::Transition(t));
      }
    }

    keep_open
  }

  /// Restores the net's marking to what it was before the route started replaying.
  pub fn close(&self, net: &mut PetriNet) {
    net.set_marking(&self.original_marking);
  }
}
