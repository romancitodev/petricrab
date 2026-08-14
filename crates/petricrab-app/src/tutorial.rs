use eframe::egui;

use crate::app::{EditMode, PetriApp, Selection};
use crate::icons;
use crate::model::{ArcKind, Marking};
use crate::theme;

struct Step {
  title: &'static str,
  body: &'static str,
}

const STEPS: &[Step] = &[
  Step {
    title: "Agregá un place",
    body: "Presioná P (o el ícono de círculo en la barra de abajo) y hacé click en el canvas para crear un place.",
  },
  Step {
    title: "Agregá una transition",
    body: "Presioná T (o el ícono de rectángulo) y hacé click en el canvas para crear una transition.",
  },
  Step {
    title: "Conectá un arco",
    body: "Presioná C (modo Conectar) y hacé click primero en el place, después en la transition, para trazar un arco entre ellos.",
  },
  Step {
    title: "Probá los tipos de arco",
    body: "Volvé a modo Seleccionar (V) y hacé click en el arco. En el panel de Selección probá Peek e Inhibit, y después dejalo de nuevo en Consume para poder simular.",
  },
  Step {
    title: "Agregá un token",
    body: "Hacé click en el place. En el panel de Selección, subí el stepper de Tokens a 1 o más.",
  },
  Step {
    title: "Dispará la transition",
    body: "Con al menos un token en el place de entrada, la transition queda habilitada. Hacé click directo sobre ella para dispararla.",
  },
  Step {
    title: "Mirá el análisis",
    body: "Abrí Ver → Propiedades del net para ver acotamiento, liveness, reversibilidad y deadlocks de tu red.",
  },
];

/// Step index that asks the user to try Peek/Inhibit on the arc — the only step whose
/// completion depends on more than the current net/app state alone (see `arc_kind_tried`).
const ARC_TYPE_STEP: usize = 3;
/// Step index that asks the user to fire the transition — the only step keyed off a marking
/// snapshot (see `entry_marking`).
const FIRE_STEP: usize = 5;

pub struct TutorialState {
  step: usize,
  /// Marking snapshot taken the moment the "fire" step starts — that step's completion is "the
  /// marking changed since we got here" rather than any specific transition firing, so it
  /// doesn't need to know which transition the user picked.
  entry_marking: Option<Marking>,
  /// Set once the arc-type step has actually seen the arc as Peek or Inhibit — completion also
  /// requires it to be back on Consume (see `body` above: simulating needs a normal arc), so a
  /// single "is it Peek/Inhibit right now" check can't tell "never touched it" from "touched it
  /// and put it back."
  arc_kind_tried: bool,
}

impl TutorialState {
  pub fn new() -> Self {
    Self {
      step: 0,
      entry_marking: None,
      arc_kind_tried: false,
    }
  }
}

/// The single arc in the net at the point the tutorial cares about it — there's only ever one
/// until "Conectá un arco" completes, so the first one found is *the* one.
fn single_arc_kind(app: &PetriApp) -> Option<ArcKind> {
  app
    .net
    .transition_ids()
    .find_map(|t| app.net.inputs(t).first().map(|&(_, kind)| kind))
}

/// Where to point the spotlight right now. For the mode-driven steps this is dynamic, not fixed
/// for the whole step: it starts on the toolbar (go pick the mode), jumps to the canvas once the
/// user switches to it (go click there), and — for the two steps whose action lives in the
/// Selección panel, not the canvas — jumps again to the dock the moment something gets selected.
/// So the highlight visibly reacts to each part of the action instead of sitting still through
/// all of it.
fn current_target(app: &PetriApp, step: usize) -> egui::Rect {
  let has_selection = !matches!(app.selection, Selection::None);
  match step {
    0 if app.mode != EditMode::AddPlace => app.toolbar_rect,
    0 => app.canvas_rect,
    1 if app.mode != EditMode::AddTransition => app.toolbar_rect,
    1 => app.canvas_rect,
    2 if app.mode != EditMode::Connect => app.toolbar_rect,
    2 => app.canvas_rect,
    ARC_TYPE_STEP | 4 if app.mode != EditMode::Select => app.toolbar_rect,
    ARC_TYPE_STEP | 4 if !has_selection => app.canvas_rect,
    ARC_TYPE_STEP | 4 => app.dock_panel_rect,
    FIRE_STEP => app.canvas_rect,
    _ => app.menu_ver_rect,
  }
}

/// Whether the action a step asks for has actually happened, checked against real net/app
/// state — no simulated clicks, just "did the thing the user was told to do leave the trace it
/// would leave."
fn step_done(app: &PetriApp, state: &TutorialState) -> bool {
  match state.step {
    0 => app.net.place_ids().count() >= 1,
    1 => app.net.transition_ids().count() >= 1,
    2 => app
      .net
      .transition_ids()
      .any(|t| !app.net.inputs(t).is_empty() || !app.net.outputs(t).is_empty()),
    ARC_TYPE_STEP => {
      state.arc_kind_tried && matches!(single_arc_kind(app), Some(ArcKind::Consume(_)))
    }
    4 => app.net.marking().values().any(|&tokens| tokens > 0),
    FIRE_STEP => state
      .entry_marking
      .as_ref()
      .is_some_and(|entry| *entry != app.net.marking()),
    _ => app.properties.is_some(),
  }
}

/// Dims the whole screen except `hole` (four strips around it, so the hole itself gets no
/// scrim) and rings the hole in the accent color — same "everything but the relevant bit stays
/// bright" idea `editor::draw_net` uses for route replay, just done as a screen-space cutout
/// since here there's no net element to key the dimming off of.
fn draw_spotlight(ctx: &egui::Context, hole: egui::Rect) {
  let screen = ctx.content_rect();
  let painter = ctx.layer_painter(egui::LayerId::new(
    egui::Order::Foreground,
    egui::Id::new("tutorial-scrim"),
  ));
  let scrim = egui::Color32::from_black_alpha(140);
  let strips = [
    egui::Rect::from_min_max(screen.min, egui::pos2(screen.max.x, hole.min.y)),
    egui::Rect::from_min_max(egui::pos2(screen.min.x, hole.max.y), screen.max),
    egui::Rect::from_min_max(
      egui::pos2(screen.min.x, hole.min.y),
      egui::pos2(hole.min.x, hole.max.y),
    ),
    egui::Rect::from_min_max(
      egui::pos2(hole.max.x, hole.min.y),
      egui::pos2(screen.max.x, hole.max.y),
    ),
  ];
  for strip in strips {
    painter.rect_filled(strip, 0.0, scrim);
  }
  painter.rect_stroke(
    hole,
    6.0,
    egui::Stroke::new(2.0, theme::accent()),
    egui::StrokeKind::Outside,
  );
}

/// Resets per-step bookkeeping that isn't derivable from `app` alone, on entering `step` —
/// called both by auto-advance and by the manual Anterior/Siguiente buttons so the two paths
/// can't disagree about what state a step starts in.
fn enter_step(state: &mut TutorialState, app: &PetriApp, step: usize) {
  state.step = step;
  if step == ARC_TYPE_STEP {
    state.arc_kind_tried = false;
  }
  if step == FIRE_STEP {
    state.entry_marking = Some(app.net.marking());
  }
}

/// Draws the current step's spotlight + instruction card, auto-advances once `step_done` says
/// the user actually did the thing, and drives the Anterior/Siguiente/Saltar override buttons.
/// Returns `false` once it should close (finished, "Saltar"/"Finalizar", or Esc) — the caller is
/// then responsible for remembering that the tutorial was seen, same take/replace pattern as
/// `route_modal::RouteModal::show`.
pub fn show(state: &mut TutorialState, ctx: &egui::Context, app: &mut PetriApp) -> bool {
  if state.step == FIRE_STEP && state.entry_marking.is_none() {
    state.entry_marking = Some(app.net.marking());
  }
  if state.step == ARC_TYPE_STEP
    && matches!(
      single_arc_kind(app),
      Some(ArcKind::Peek(_) | ArcKind::Inhibit(_))
    )
  {
    state.arc_kind_tried = true;
  }

  if step_done(app, state) {
    if state.step + 1 == STEPS.len() {
      return false;
    }
    enter_step(state, app, state.step + 1);
  }

  let step = &STEPS[state.step];
  draw_spotlight(ctx, current_target(app, state.step));

  let mut keep_open = true;
  ctx.input(|i| {
    if i.key_pressed(egui::Key::Escape) {
      keep_open = false;
    }
  });

  egui::Area::new(egui::Id::new("tutorial-popup"))
    // Strictly above `Order::Foreground`, where the scrim paints — same tier ties are not a
    // reliable stacking order in egui, so the card must sit in the tier above it, not beside it.
    .order(egui::Order::Tooltip)
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
          ui.set_width(320.0);
          ui.horizontal(|ui| {
            ui.label(icons::icon("graduation-cap", 14.0));
            ui.strong(step.title);
            ui.weak(format!("paso {}/{}", state.step + 1, STEPS.len()));
          });
          ui.add_space(8.0);
          ui.label(step.body);
          ui.add_space(10.0);

          ui.horizontal(|ui| {
            if ui
              .add_enabled(state.step > 0, egui::Button::new("Anterior"))
              .clicked()
            {
              enter_step(state, app, state.step - 1);
            }
            let next_label = if state.step + 1 == STEPS.len() {
              "Finalizar"
            } else {
              "Siguiente"
            };
            if ui.button(next_label).clicked() {
              if state.step + 1 == STEPS.len() {
                keep_open = false;
              } else {
                enter_step(state, app, state.step + 1);
              }
            }
            if ui.button("Saltar").clicked() {
              keep_open = false;
            }
          });
          ui.add_space(4.0);
          ui.weak("Avanza solo al hacer la acción pedida. Esc para salir.");
        });
    });

  keep_open
}
