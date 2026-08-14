use std::collections::HashMap;

use eframe::egui;

use crate::app::{NodeId, PetriApp, Selection};
use crate::model::{PetriNet, PlaceId, TransitionId};

/// Everything Ctrl+Z should be able to bring back: the net itself, node positions, transition
/// rotations, place colors and notes. `undo_stack`/`redo_stack` on `PetriApp` hold these.
pub(crate) struct Snapshot {
  net: PetriNet,
  positions: HashMap<NodeId, egui::Pos2>,
  rotation: HashMap<TransitionId, f32>,
  colors: HashMap<PlaceId, egui::Color32>,
  notes: slotmap::SlotMap<crate::app::NoteId, crate::app::NoteData>,
}

impl Snapshot {
  fn capture(app: &PetriApp) -> Self {
    Self {
      net: app.net.clone(),
      positions: app.positions.clone(),
      rotation: app.rotation.clone(),
      colors: app.colors.clone(),
      notes: app.notes.clone(),
    }
  }

  fn restore(self, app: &mut PetriApp) {
    app.net = self.net;
    app.positions = self.positions;
    app.rotation = self.rotation;
    app.colors = self.colors;
    app.notes = self.notes;
    app.selection = Selection::None;
    app.selection_focus = None;
  }
}

const MAX_UNDO_STEPS: usize = 100;

/// Saves an undo point capturing the state *before* the mutation that's about to happen, and
/// drops the redo stack (a fresh edit invalidates whatever redo history there was). Called at
/// the start of every action that changes the net, positions, rotation, colors or notes — plain
/// text edits (labels, note text) are the deliberate exception, see the call sites, since
/// checkpointing every keystroke would make one undo step per character typed.
pub(crate) fn checkpoint(app: &mut PetriApp) {
  app.redo_stack.clear();
  app.undo_stack.push(Snapshot::capture(app));
  if app.undo_stack.len() > MAX_UNDO_STEPS {
    app.undo_stack.remove(0);
  }
}

pub(crate) fn undo(app: &mut PetriApp) {
  let Some(prev) = app.undo_stack.pop() else {
    return;
  };
  app.redo_stack.push(Snapshot::capture(app));
  prev.restore(app);
}

pub(crate) fn redo(app: &mut PetriApp) {
  let Some(next) = app.redo_stack.pop() else {
    return;
  };
  app.undo_stack.push(Snapshot::capture(app));
  next.restore(app);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn undo_redo_roundtrips_a_checkpointed_change() {
    let mut app = PetriApp::new();
    checkpoint(&mut app);
    let id = app.net.add_place("p1");
    assert_eq!(app.net.place_ids().count(), 1);

    undo(&mut app);
    assert_eq!(app.net.place_ids().count(), 0);

    redo(&mut app);
    assert_eq!(app.net.place_ids().count(), 1);
    assert_eq!(app.net.place_label(id), "p1");
  }
}
