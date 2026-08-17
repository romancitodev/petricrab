//! The DSL dock tab: a text buffer (`app.dsl.source`, persisted in the `.gpn` project like
//! everything else on the document) with an "Aplicar" button that replaces `app.net` wholesale,
//! plus a read-only projection of the resulting net as the pre/post-condition table the
//! assignment asks for.
use std::collections::HashMap;

use eframe::egui;

use crate::app::{NodeId, PetriApp, Selection};
use crate::dsl::{self, DslError};
use crate::editor::{card, checkpoint, section_label};
use crate::model::PetriNet;

pub struct DslState {
  pub source: String,
  pub errors: Vec<DslError>,
}

impl DslState {
  pub fn new(net: &PetriNet) -> Self {
    Self {
      source: dsl::to_dsl(net),
      errors: Vec::new(),
    }
  }
}

const COL_GAP: f32 = 320.0;
const ROW_GAP: f32 = 80.0;

/// Places in one column, transitions in another, both in declaration order. Dumb and correct —
/// no force-directed layout, just enough for the result to be visible and untangled on the
/// canvas right after "Aplicar".
fn autolayout(net: &PetriNet) -> HashMap<NodeId, egui::Pos2> {
  let mut positions = HashMap::new();
  for (i, p) in net.place_ids().enumerate() {
    positions.insert(NodeId::Place(p), egui::pos2(0.0, i as f32 * ROW_GAP));
  }
  for (i, t) in net.transition_ids().enumerate() {
    positions.insert(
      NodeId::Transition(t),
      egui::pos2(COL_GAP, i as f32 * ROW_GAP),
    );
  }
  positions
}

pub fn show(app: &mut PetriApp, ui: &mut egui::Ui) {
  ui.horizontal(|ui| {
    if ui.button("Aplicar").clicked() {
      match dsl::parse(&app.dsl.source) {
        Ok(net) => {
          checkpoint(app);
          app.positions = autolayout(&net);
          app.net = net;
          app.colors.clear();
          app.rotation.clear();
          app.selection = Selection::None;
          app.selection_focus = None;
          app.dsl.errors.clear();
          app.notify(egui_toast::ToastKind::Success, "DSL aplicado");
        }
        Err(errors) => app.dsl.errors = errors,
      }
    }
    if ui.button("Copiar como Markdown").clicked() {
      ui.ctx().copy_text(dsl::to_markdown(&app.net));
    }
  });
  ui.add_space(8.0);

  ui.add(
    egui::TextEdit::multiline(&mut app.dsl.source)
      .code_editor()
      .desired_width(f32::INFINITY)
      .desired_rows(16),
  );

  if !app.dsl.errors.is_empty() {
    ui.add_space(8.0);
    card(ui, |ui| {
      for e in &app.dsl.errors {
        ui.colored_label(
          ui.visuals().warn_fg_color,
          format!("línea {}: {}", e.line, e.message),
        );
      }
    });
  }

  ui.add_space(12.0);
  ui.separator();
  ui.add_space(10.0);

  section_label(ui, "Lugares");
  ui.add_space(6.0);
  egui::Grid::new("dsl_places_grid")
    .num_columns(2)
    .spacing([12.0, 4.0])
    .striped(true)
    .show(ui, |ui| {
      for p in app.net.place_ids() {
        ui.label(app.net.place_label(p));
        ui.label(format!("{} tok.", app.net.tokens(p)));
        ui.end_row();
      }
    });
  ui.add_space(10.0);

  section_label(ui, "Transiciones (pre / post)");
  ui.add_space(6.0);
  egui::Grid::new("dsl_transitions_grid")
    .num_columns(3)
    .spacing([12.0, 4.0])
    .striped(true)
    .show(ui, |ui| {
      ui.strong("Transición");
      ui.strong("Entradas");
      ui.strong("Salidas");
      ui.end_row();
      for t in app.net.transition_ids() {
        let ins = app
          .net
          .inputs(t)
          .iter()
          .map(|&(p, _)| app.net.place_label(p))
          .collect::<Vec<_>>()
          .join(", ");
        let outs = app
          .net
          .outputs(t)
          .iter()
          .map(|&(p, _)| app.net.place_label(p))
          .collect::<Vec<_>>()
          .join(", ");
        ui.label(app.net.transition_label(t));
        ui.label(if ins.is_empty() { "—" } else { &ins });
        ui.label(if outs.is_empty() { "—" } else { &outs });
        ui.end_row();
      }
    });
}
