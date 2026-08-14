use eframe::egui;

use crate::app::{EditMode, NodeId, PetriApp, Selection};
use crate::icons;
use crate::model::{ArcKind, fire};
use crate::theme;

use super::align::{Align, align_selected, beautify};
use super::canvas::{
  apply_mode, center_on_node, delete_selected, fire_step, reset_sim, step_back, step_forward,
  toggle_simulate,
};
use super::geometry::{nudge_rotation, set_rotation, transition_angle};

pub(crate) fn section_label(ui: &mut egui::Ui, text: &str) {
  ui.label(
    egui::RichText::new(text.to_uppercase())
      .size(11.0)
      .color(ui.visuals().weak_text_color())
      .strong(),
  );
}

/// A grouped, rounded card matching the toolbar/simulate-popup look, used to visually separate
/// inspector sections from the flat panel background instead of leaving controls floating loose.
pub(crate) fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
  egui::Frame::default()
    .fill(ui.visuals().faint_bg_color)
    .corner_radius(10.0)
    .inner_margin(egui::Margin::symmetric(12, 10))
    .show(ui, add_contents);
}

fn destructive_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
  let danger = theme::danger();
  ui.add(
    egui::Button::new((
      icons::icon("trash-2", 14.0).color(danger),
      egui::RichText::new(label).color(danger),
    ))
    .corner_radius(6.0)
    .stroke(egui::Stroke::new(1.0, danger.gamma_multiply(0.4))),
  )
  .on_hover_text("Supr")
}

/// Inspector header: a round icon badge (not a boxed card, just an avatar) next to the
/// entity's editable name and its kind, e.g. a place's filled-circle glyph beside "p1" / "Place".
fn entity_title(ui: &mut egui::Ui, icon_name: &'static str, name: &mut String, kind: &str) {
  ui.horizontal(|ui| {
    egui::Frame::default()
      .fill(ui.visuals().faint_bg_color)
      .corner_radius(14.0)
      .inner_margin(egui::Margin::same(7))
      .show(ui, |ui| {
        ui.label(icons::icon(icon_name, 15.0));
      });
    ui.add_space(4.0);
    ui.vertical(|ui| {
      ui.add(
        egui::TextEdit::singleline(name)
          .font(egui::FontId::proportional(15.0))
          .frame(egui::Frame::NONE)
          .desired_width(140.0),
      );
      ui.weak(kind);
    });
  });
}

/// `a (icon) b` row, used instead of a raw unicode arrow character for arc summaries.
pub(crate) fn arrow_row(ui: &mut egui::Ui, a: &str, b: &str) {
  ui.horizontal(|ui| {
    ui.label(egui::RichText::new(a).strong());
    ui.label(icons::icon("arrow-right", 12.0));
    ui.label(egui::RichText::new(b).strong());
  });
}

/// A labeled +/- stepper instead of a bare debug-looking number field. Returns true on change.
fn stepper(ui: &mut egui::Ui, label: &str, value: &mut u32, min: u32, max: u32) -> bool {
  let mut changed = false;
  ui.horizontal(|ui| {
    ui.label(label);
    let step_btn = |ui: &mut egui::Ui, icon_name: &'static str| {
      ui.add(
        egui::Button::new(icons::icon(icon_name, 12.0))
          .corner_radius(4.0)
          .min_size(egui::vec2(28.0, 24.0)),
      )
    };
    if step_btn(ui, "minus").clicked() && *value > min {
      *value -= 1;
      changed = true;
    }
    ui.label(egui::RichText::new(value.to_string()).monospace().strong());
    if step_btn(ui, "plus").clicked() && *value < max {
      *value += 1;
      changed = true;
    }
  });
  changed
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ArcKindTag {
  Consume,
  Peek,
  Inhibit,
}

impl ArcKindTag {
  pub(crate) fn from_kind(kind: ArcKind) -> Self {
    match kind {
      ArcKind::Consume(_) => Self::Consume,
      ArcKind::Peek(_) => Self::Peek,
      ArcKind::Inhibit(_) => Self::Inhibit,
    }
  }

  pub(crate) fn to_kind(self, weight: u32) -> ArcKind {
    match self {
      Self::Consume => ArcKind::Consume(weight),
      Self::Peek => ArcKind::Peek(weight),
      Self::Inhibit => ArcKind::Inhibit(weight),
    }
  }

  pub(crate) fn icon(self) -> &'static str {
    match self {
      Self::Consume => "arrow-right",
      Self::Peek => "eye",
      Self::Inhibit => "ban",
    }
  }

  pub(crate) fn tooltip(self) -> &'static str {
    match self {
      Self::Consume => "Consume: resta tokens al disparar",
      Self::Peek => "Peek: requiere tokens pero no los consume",
      Self::Inhibit => "Inhibit: bloquea mientras haya tokens",
    }
  }
}

fn arc_in_editor(
  app: &mut PetriApp,
  ui: &mut egui::Ui,
  p: crate::model::PlaceId,
  t: crate::model::TransitionId,
) {
  let Some(&(_, current)) = app.net.inputs(t).iter().find(|(place, _)| *place == p) else {
    return;
  };
  let mut tag = ArcKindTag::from_kind(current);
  let mut weight = current.weight();
  let mut changed = false;

  ui.horizontal(|ui| {
    for candidate in [ArcKindTag::Consume, ArcKindTag::Peek, ArcKindTag::Inhibit] {
      let btn = egui::Button::new(icons::icon(candidate.icon(), 15.0))
        .selected(tag == candidate)
        .corner_radius(6.0)
        .min_size(egui::vec2(36.0, 30.0));
      if ui.add(btn).on_hover_text(candidate.tooltip()).clicked() {
        tag = candidate;
        changed = true;
      }
    }
  });
  // ponytail: petricrab-core's analysis bridge only models Peek/Inhibit as Murata's classic
  // unweighted arcs ("some token" / "no token"); Consume is the only kind with a real weight
  // there. Capping the stepper at 1 for Peek/Inhibit keeps every arc the editor can create
  // faithfully representable by the analysis (see analysis.rs::to_analysis_net). Upgrade path:
  // weighted Peek/Inhibit in petricrab-core if this cap ever needs to go.
  let max_weight = if tag == ArcKindTag::Consume { 99 } else { 1 };
  if weight > max_weight {
    weight = max_weight;
    changed = true;
  }
  if stepper(ui, "Peso", &mut weight, 1, max_weight) {
    changed = true;
  }

  if changed {
    app.net.remove_arc_place_to_transition(p, t);
    let _ = app
      .net
      .add_arc_place_to_transition(p, t, tag.to_kind(weight));
  }
}

fn arc_out_editor(
  app: &mut PetriApp,
  ui: &mut egui::Ui,
  t: crate::model::TransitionId,
  p: crate::model::PlaceId,
) {
  let Some(&(_, current_weight)) = app.net.outputs(t).iter().find(|(place, _)| *place == p) else {
    return;
  };
  let mut weight = current_weight;
  if stepper(ui, "Peso", &mut weight, 1, 99) {
    app.net.remove_arc_transition_to_place(t, p);
    let _ = app.net.add_arc_transition_to_place(t, p, weight);
  }
}

pub fn selection_panel(app: &mut PetriApp, ui: &mut egui::Ui) {
  section_label(ui, "Selección");
  ui.add_space(8.0);

  // Cloned once so the rest of this function can freely call `&mut app.*` (e.g.
  // `delete_selected`, the arc editors) without fighting a live borrow of `app.selection`.
  let selection = app.selection.clone();
  match selection {
    Selection::Nodes(nodes) if nodes.len() == 1 => {
      let node = *nodes.iter().next().unwrap();
      match node {
        NodeId::Place(p) if app.net.place_ids().any(|id| id == p) => {
          entity_title(ui, "circle", app.net.place_label_mut(p).unwrap(), "Place");
          ui.add_space(12.0);
          ui.separator();
          ui.add_space(10.0);
          let mut tokens = app.net.tokens(p);
          if stepper(ui, "Tokens", &mut tokens, 0, 999) {
            app.net.set_tokens(p, tokens);
          }
          ui.add_space(10.0);
          ui.horizontal(|ui| {
            ui.label("Color");
            let mut color = app.colors.get(&p).copied().unwrap_or(theme::ink());
            if ui.color_edit_button_srgba(&mut color).changed() {
              app.colors.insert(p, color);
            }
            if app.colors.contains_key(&p)
              && ui
                .add(egui::Button::new(icons::icon("rotate-ccw", 13.0)).frame(false))
                .on_hover_text("Restablecer color")
                .clicked()
            {
              app.colors.remove(&p);
            }
          });
          ui.add_space(14.0);
          if destructive_button(ui, "Eliminar").clicked() {
            delete_selected(app);
          }
        }
        NodeId::Transition(t) if app.net.transition_ids().any(|id| id == t) => {
          entity_title(
            ui,
            "rectangle-vertical",
            app.net.transition_label_mut(t).unwrap(),
            "Transition",
          );
          ui.add_space(12.0);
          ui.separator();
          ui.add_space(10.0);
          rotation_control(ui, app, t);
          ui.add_space(14.0);
          if destructive_button(ui, "Eliminar").clicked() {
            delete_selected(app);
          }
        }
        _ => {
          app.selection = Selection::None;
          ui.weak("Nada seleccionado");
        }
      }
    }
    Selection::Nodes(nodes) if nodes.len() > 1 => {
      ui.label(egui::RichText::new(format!("{} elementos", nodes.len())).strong());
      ui.weak("Tocá un elemento para editarlo");
      ui.add_space(10.0);
      ui.separator();
      ui.add_space(10.0);

      section_label(ui, "Alinear");
      ui.add_space(6.0);
      ui.horizontal(|ui| {
        let align_btn = |ui: &mut egui::Ui, icon_name: &'static str, tooltip: &str| {
          ui.add(
            egui::Button::new(icons::icon(icon_name, 14.0))
              .corner_radius(6.0)
              .min_size(egui::vec2(36.0, 30.0)),
          )
          .on_hover_text(tooltip)
        };
        if align_btn(ui, "wand-sparkles", "Auto: reordena y espacia parejo").clicked() {
          align_selected(app, &nodes, Align::Auto);
        }
        if align_btn(ui, "align-start-vertical", "Izquierda").clicked() {
          align_selected(app, &nodes, Align::Left);
        }
        if align_btn(ui, "align-center-vertical", "Centro").clicked() {
          align_selected(app, &nodes, Align::Center);
        }
        if align_btn(ui, "align-end-vertical", "Derecha").clicked() {
          align_selected(app, &nodes, Align::Right);
        }
        if align_btn(ui, "align-start-horizontal", "Arriba").clicked() {
          align_selected(app, &nodes, Align::Top);
        }
        if align_btn(ui, "align-center-horizontal", "Medio").clicked() {
          align_selected(app, &nodes, Align::Middle);
        }
        if align_btn(ui, "align-end-horizontal", "Abajo").clicked() {
          align_selected(app, &nodes, Align::Bottom);
        }
      });
      ui.add_space(8.0);
      ui.horizontal(|ui| {
        ui.weak("Espaciado (Auto)");
        ui.add(
          egui::DragValue::new(&mut app.align_gap)
            .range(16.0..=400.0)
            .suffix(" px"),
        )
        .on_hover_text("Distancia entre elementos consecutivos cuando alineás con Auto");
      });
      ui.add_space(10.0);
      ui.separator();
      ui.add_space(10.0);

      if let Some(focus) = app.selection_focus {
        if !nodes.contains(&focus) {
          app.selection_focus = None;
        }
      }

      let mut sorted: Vec<NodeId> = nodes.iter().copied().collect();
      sorted.sort();
      for node in sorted {
        let (icon_name, exists): (&'static str, bool) = match node {
          NodeId::Place(p) => ("circle", app.net.place_ids().any(|id| id == p)),
          NodeId::Transition(t) => (
            "rectangle-vertical",
            app.net.transition_ids().any(|id| id == t),
          ),
        };
        // Deleted mid-selection: just skip it, the entry drops out on the next click.
        if !exists {
          continue;
        }
        card(ui, |ui| {
          let focused = app.selection_focus == Some(node);
          ui.horizontal(|ui| {
            let toggle = ui.add(
              egui::Button::new(icons::icon(icon_name, 13.0))
                .frame(false)
                .selected(focused),
            );
            if toggle.clicked() {
              app.selection_focus = if focused { None } else { Some(node) };
            }
            match node {
              NodeId::Place(p) => {
                ui.add(
                  egui::TextEdit::singleline(app.net.place_label_mut(p).unwrap())
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
                );
              }
              NodeId::Transition(t) => {
                ui.add(
                  egui::TextEdit::singleline(app.net.transition_label_mut(t).unwrap())
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
                );
              }
            }
          });
          if focused {
            ui.add_space(8.0);
            match node {
              NodeId::Place(p) => {
                let mut tokens = app.net.tokens(p);
                if stepper(ui, "Tokens", &mut tokens, 0, 999) {
                  app.net.set_tokens(p, tokens);
                }
              }
              NodeId::Transition(t) => {
                rotation_control(ui, app, t);
              }
            }
          }
        });
        ui.add_space(6.0);
      }

      ui.add_space(8.0);
      if destructive_button(ui, "Eliminar todos").clicked() {
        delete_selected(app);
        app.selection_focus = None;
      }
    }
    Selection::ArcIn(p, t) => {
      arrow_row(ui, app.net.place_label(p), app.net.transition_label(t));
      ui.weak("Arco de entrada");
      ui.add_space(10.0);
      ui.separator();
      ui.add_space(10.0);
      arc_in_editor(app, ui, p, t);
      ui.add_space(14.0);
      if destructive_button(ui, "Eliminar arco").clicked() {
        delete_selected(app);
      }
    }
    Selection::ArcOut(t, p) => {
      arrow_row(ui, app.net.transition_label(t), app.net.place_label(p));
      ui.weak("Arco de salida");
      ui.add_space(10.0);
      ui.separator();
      ui.add_space(10.0);
      arc_out_editor(app, ui, t, p);
      ui.add_space(14.0);
      if destructive_button(ui, "Eliminar arco").clicked() {
        delete_selected(app);
      }
    }
    Selection::Note(id) if app.notes.contains_key(id) => {
      ui.horizontal(|ui| {
        ui.label(icons::icon("sticky-note", 15.0));
        ui.label(egui::RichText::new("Nota").strong());
      });
      ui.add_space(12.0);
      ui.separator();
      ui.add_space(10.0);
      if let Some(note) = app.notes.get_mut(id) {
        ui.add(
          egui::TextEdit::multiline(&mut note.text)
            .desired_rows(6)
            .desired_width(f32::INFINITY)
            .hint_text("Escribí lo que quieras…"),
        );
      }
      ui.add_space(10.0);
      ui.horizontal(|ui| {
        ui.label("Color");
        let mut color = app
          .notes
          .get(id)
          .and_then(|n| n.color)
          .unwrap_or(theme::surface_raised());
        if ui.color_edit_button_srgba(&mut color).changed()
          && let Some(note) = app.notes.get_mut(id)
        {
          note.color = Some(color);
        }
        if app.notes.get(id).is_some_and(|n| n.color.is_some())
          && ui
            .add(egui::Button::new(icons::icon("rotate-ccw", 13.0)).frame(false))
            .on_hover_text("Restablecer color")
            .clicked()
          && let Some(note) = app.notes.get_mut(id)
        {
          note.color = None;
        }
      });
      ui.add_space(14.0);
      if destructive_button(ui, "Eliminar").clicked() {
        delete_selected(app);
      }
    }
    _ => {
      ui.weak("Nada seleccionado");
    }
  }
}

/// Full list of the net's places/transitions, click-to-select-and-center — the "where is
/// everything" view for nets too big to eyeball on the canvas at a glance.
pub fn outline_panel(app: &mut PetriApp, ui: &mut egui::Ui) {
  egui::ScrollArea::vertical().show(ui, |ui| {
    section_label(ui, "Places");
    ui.add_space(4.0);
    let place_ids: Vec<_> = app.net.place_ids().collect();
    if place_ids.is_empty() {
      ui.weak("(ninguno)");
    }
    for p in place_ids {
      let selected = app.selection == Selection::Nodes([NodeId::Place(p)].into());
      let row = ui.add(
        egui::Button::new(format!(
          "{}   ·   {} tok.",
          app.net.place_label(p),
          app.net.tokens(p)
        ))
        .frame(false)
        .selected(selected)
        .min_size(egui::vec2(ui.available_width(), 0.0)),
      );
      if row.clicked() {
        app.selection = Selection::Nodes([NodeId::Place(p)].into());
        center_on_node(app, NodeId::Place(p));
      }
    }
    ui.add_space(10.0);

    section_label(ui, "Transitions");
    ui.add_space(4.0);
    let marking = app.net.marking();
    let enabled = fire::enabled_transitions(&app.net, &marking);
    let transition_ids: Vec<_> = app.net.transition_ids().collect();
    if transition_ids.is_empty() {
      ui.weak("(ninguna)");
    }
    for t in transition_ids {
      let selected = app.selection == Selection::Nodes([NodeId::Transition(t)].into());
      let icon_name = if enabled.contains(&t) { "zap" } else { "ban" };
      let row = ui.add(
        egui::Button::new((icons::icon(icon_name, 13.0), app.net.transition_label(t)))
          .frame(false)
          .selected(selected)
          .min_size(egui::vec2(ui.available_width(), 0.0)),
      );
      if row.clicked() {
        app.selection = Selection::Nodes([NodeId::Transition(t)].into());
        center_on_node(app, NodeId::Transition(t));
      }
    }
  });
}

fn mode_icon_and_tooltip(mode: EditMode) -> (&'static str, &'static str) {
  match mode {
    EditMode::Select => ("mouse-pointer-2", "Seleccionar / token game (V)"),
    EditMode::AddPlace => ("circle", "Agregar place (P)"),
    EditMode::AddTransition => ("rectangle-vertical", "Agregar transition (T)"),
    EditMode::Connect => ("cable", "Conectar arco (C)"),
    EditMode::AddNote => ("sticky-note", "Agregar nota (N)"),
  }
}

pub fn toolbar(app: &mut PetriApp, ui: &mut egui::Ui) {
  ui.horizontal(|ui| {
    for mode in [
      EditMode::Select,
      EditMode::AddPlace,
      EditMode::AddTransition,
      EditMode::Connect,
      EditMode::AddNote,
    ] {
      let (icon_name, tooltip) = mode_icon_and_tooltip(mode);
      let button = egui::Button::new(icons::icon(icon_name, 17.0))
        .selected(app.mode == mode)
        .corner_radius(8.0)
        .min_size(egui::vec2(38.0, 32.0));
      if ui.add(button).on_hover_text(tooltip).clicked() {
        apply_mode(app, mode);
      }
    }

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);

    let beautify_button = egui::Button::new(icons::icon("sparkles", 17.0))
      .corner_radius(8.0)
      .min_size(egui::vec2(38.0, 32.0));
    let selected = match &app.selection {
      Selection::Nodes(n) => n.clone(),
      _ => std::collections::HashSet::new(),
    };
    let tooltip = if selected.len() > 1 {
      "Beautify: reacomoda la selección para que se lea más cómodo"
    } else {
      "Beautify: reacomoda todo el net para que se lea más cómodo"
    };
    if ui.add(beautify_button).on_hover_text(tooltip).clicked() {
      beautify(app, &selected);
    }

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);

    let simulate_button = egui::Button::new(icons::icon("play", 17.0))
      .selected(app.simulate_open)
      .corner_radius(8.0)
      .min_size(egui::vec2(38.0, 32.0));
    if ui.add(simulate_button).on_hover_text("Simular").clicked() {
      toggle_simulate(app);
    }
  });
}

pub(crate) fn token_chip(ui: &mut egui::Ui, place_label: &str, tokens: u32) {
  egui::Frame::default()
    .fill(ui.visuals().extreme_bg_color)
    .corner_radius(10.0)
    .inner_margin(egui::Margin::symmetric(8, 3))
    .show(ui, |ui| {
      // Extend, not the `horizontal_wrapped` default Wrap: otherwise long text wraps letter by
      // letter inside the chip instead of the whole chip moving to the next row.
      ui.add(
        egui::Label::new(egui::RichText::new(format!("{place_label}  {tokens}")).monospace())
          .wrap_mode(egui::TextWrapMode::Extend),
      );
    });
}

/// One chip per marked place, wrapped onto new rows as needed. The one place that renders a
/// full `Marking` as chips instead of a flat comma-joined string, so each label/count pair stays
/// visually paired even when it wraps.
pub(crate) fn marking_chips(
  ui: &mut egui::Ui,
  net: &crate::model::PetriNet,
  marking: &crate::model::Marking,
) {
  ui.horizontal_wrapped(|ui| {
    let mut any = false;
    for (&place, &tokens) in marking.iter() {
      if tokens > 0 {
        any = true;
        token_chip(ui, net.place_label(place), tokens);
      }
    }
    if !any {
      ui.weak("(vacío)");
    }
  });
}

/// Step-through simulator: shows the live net's current marking as token chips, its enabled
/// transitions as one-click fire buttons, and back/reset/forward controls over an undo/redo
/// history. It's the same firing rule the reachability graph explores and the canvas
/// token-game uses — this is just a docked, friendlier way to drive it.
pub fn simulate_panel(app: &mut PetriApp, ui: &mut egui::Ui) {
  // A fixed (not max) width keeps every row's `available_width()` identical across frames;
  // `set_max_width` let the full-width "fire" button and the control row disagree on width
  // between passes, which is what made the whole popup render off-center.
  ui.set_width(280.0);
  ui.horizontal(|ui| {
    ui.label(icons::icon("play", 14.0));
    ui.strong("Simulación");
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
      if ui
        .add(egui::Button::new(icons::icon("x", 13.0)).frame(false))
        .on_hover_text("Cerrar simulación")
        .clicked()
      {
        toggle_simulate(app);
      }
    });
  });
  ui.weak(format!(
    "Paso {} · {} adelante disponible{}",
    app.sim_history.len(),
    app.sim_future.len(),
    if app.sim_future.len() == 1 { "" } else { "s" }
  ));
  ui.add_space(12.0);

  let ctrl_btn = |icon_name: &'static str| {
    egui::Button::new(icons::icon(icon_name, 15.0))
      .corner_radius(6.0)
      .min_size(egui::vec2(36.0, 30.0))
  };
  ui.vertical_centered(|ui| {
    ui.horizontal(|ui| {
      let can_back = !app.sim_history.is_empty();
      let can_forward = !app.sim_future.is_empty();
      let can_reset = app.sim_initial.is_some();

      if ui
        .add_enabled(can_back, ctrl_btn("skip-back"))
        .on_hover_text("Paso atrás")
        .clicked()
      {
        step_back(app);
      }
      if ui
        .add_enabled(can_reset, ctrl_btn("rotate-ccw"))
        .on_hover_text("Reiniciar")
        .clicked()
      {
        reset_sim(app);
      }
      if ui
        .add_enabled(can_forward, ctrl_btn("skip-forward"))
        .on_hover_text("Paso adelante")
        .clicked()
      {
        step_forward(app);
      }
    });
  });
  ui.add_space(14.0);

  let marking = app.net.marking();
  let total_tokens: u32 = marking.values().sum();
  section_label(ui, &format!("Marking · {total_tokens} tokens"));
  ui.add_space(6.0);
  card(ui, |ui| {
    marking_chips(ui, &app.net, &marking);
  });
  ui.add_space(12.0);

  let enabled = fire::enabled_transitions(&app.net, &marking);
  section_label(ui, &format!("Transiciones habilitadas · {}", enabled.len()));
  ui.add_space(6.0);
  card(ui, |ui| {
    if enabled.is_empty() {
      ui.weak("(ninguna)");
    } else {
      for t in enabled {
        let btn = egui::Button::new((
          icons::icon("zap", 13.0),
          app.net.transition_label(t).to_string(),
        ))
        .corner_radius(6.0);
        if ui.add_sized([ui.available_width(), 28.0], btn).clicked() {
          fire_step(app, t);
        }
      }
    }
  });
}

/// A "Rotación [ 45°]" row: free-form drag/type field plus a `+45°` shortcut button.
pub(crate) fn rotation_control(
  ui: &mut egui::Ui,
  app: &mut PetriApp,
  t: crate::model::TransitionId,
) {
  ui.horizontal(|ui| {
    ui.label("Rotación");
    let mut angle = transition_angle(app, t);
    if ui
      .add(
        egui::DragValue::new(&mut angle)
          .range(0.0..=359.0)
          .suffix("°")
          .speed(1.0),
      )
      .changed()
    {
      set_rotation(app, t, angle);
    }
    if ui
      .add(
        egui::Button::new(icons::icon("rotate-cw", 12.0))
          .corner_radius(4.0)
          .min_size(egui::vec2(28.0, 24.0)),
      )
      .on_hover_text("+45°")
      .clicked()
    {
      nudge_rotation(app, t, 45.0);
    }
  });
}
