use std::collections::{HashMap, HashSet};

use eframe::egui;

use crate::app::{NodeId, PetriApp, Selection};
use crate::model::ArcKind;

use super::geometry::GRID_SPACING;
use super::history::checkpoint;

#[derive(Clone)]
enum ClipboardNode {
  Place {
    label: String,
    tokens: u32,
    color: Option<egui::Color32>,
  },
  Transition {
    label: String,
    rotation: Option<f32>,
  },
}

/// A copied selection, ready to paste: the nodes themselves plus the arcs that ran strictly
/// between two copied nodes. An arc to a node outside the selection is dropped — pasting a
/// partial subgraph can't recreate a connection to something that wasn't copied. Nodes keep
/// their original `NodeId` here only so `paste_clipboard` can remap arc endpoints to the freshly
/// created ids; those ids are otherwise meaningless once copied.
#[derive(Clone)]
pub(crate) struct Clipboard {
  nodes: Vec<(NodeId, ClipboardNode, egui::Pos2)>,
  arcs_in: Vec<(NodeId, NodeId, ArcKind)>,
  arcs_out: Vec<(NodeId, NodeId, u32)>,
}

/// `ctx` is only used to seed the OS clipboard with a marker string (see the comment on
/// `app.clipboard`'s Ctrl+V handling in `canvas` for why that matters) — the actual copied data
/// lives in `app.clipboard`, not on the OS clipboard.
pub(crate) fn copy_selection(app: &mut PetriApp, ctx: &egui::Context) {
  let Selection::Nodes(selected) = &app.selection else {
    return;
  };
  if selected.is_empty() {
    return;
  }
  let selected = selected.clone();

  let mut nodes = Vec::new();
  for &id in &selected {
    let Some(&pos) = app.positions.get(&id) else {
      continue;
    };
    let data = match id {
      NodeId::Place(p) if app.net.place_ids().any(|x| x == p) => ClipboardNode::Place {
        label: app.net.place_label(p).to_string(),
        tokens: app.net.tokens(p),
        color: app.colors.get(&p).copied(),
      },
      NodeId::Transition(t) if app.net.transition_ids().any(|x| x == t) => {
        ClipboardNode::Transition {
          label: app.net.transition_label(t).to_string(),
          rotation: app.rotation.get(&t).copied(),
        }
      }
      _ => continue,
    };
    nodes.push((id, data, pos));
  }
  if nodes.is_empty() {
    return;
  }

  let mut arcs_in = Vec::new();
  let mut arcs_out = Vec::new();
  for &id in &selected {
    if let NodeId::Transition(t) = id {
      for &(p, kind) in app.net.inputs(t) {
        if selected.contains(&NodeId::Place(p)) {
          arcs_in.push((NodeId::Place(p), NodeId::Transition(t), kind));
        }
      }
      for &(p, weight) in app.net.outputs(t) {
        if selected.contains(&NodeId::Place(p)) {
          arcs_out.push((NodeId::Transition(t), NodeId::Place(p), weight));
        }
      }
    }
  }
  app.clipboard = Some(Clipboard {
    nodes,
    arcs_in,
    arcs_out,
  });
  // egui-winit turns Ctrl+V into an `Event::Paste` only when the OS clipboard is non-empty (it
  // reads the OS clipboard itself to build that event) — an empty OS clipboard means Ctrl+V
  // produces no event at all, so our own paste handler never even gets a chance to run. Writing
  // a marker here (its contents are never read back) guarantees Ctrl+V always fires the event.
  ctx.copy_text("petricrab-clipboard".to_string());
}

/// World-space offset applied to a pasted selection so it lands next to the original instead of
/// exactly on top of it.
const PASTE_OFFSET: egui::Vec2 = egui::vec2(GRID_SPACING, GRID_SPACING);

pub(crate) fn paste_clipboard(app: &mut PetriApp) {
  let Some(clip) = app.clipboard.clone() else {
    return;
  };
  checkpoint(app);

  let mut mapping: HashMap<NodeId, NodeId> = HashMap::new();
  let mut new_selection = HashSet::new();
  for (old_id, data, pos) in &clip.nodes {
    let new_id = match data {
      ClipboardNode::Place {
        label,
        tokens,
        color,
      } => {
        app.next_place_n += 1;
        let id = app.net.add_place(label.clone());
        app.net.set_tokens(id, *tokens);
        if let Some(c) = color {
          app.colors.insert(id, *c);
        }
        NodeId::Place(id)
      }
      ClipboardNode::Transition { label, rotation } => {
        app.next_transition_n += 1;
        let id = app.net.add_transition(label.clone());
        if let Some(r) = rotation {
          app.rotation.insert(id, *r);
        }
        NodeId::Transition(id)
      }
    };
    app.positions.insert(new_id, *pos + PASTE_OFFSET);
    mapping.insert(*old_id, new_id);
    new_selection.insert(new_id);
  }
  for (from, to, kind) in &clip.arcs_in {
    if let (Some(&NodeId::Place(np)), Some(&NodeId::Transition(nt))) =
      (mapping.get(from), mapping.get(to))
    {
      let _ = app.net.add_arc_place_to_transition(np, nt, *kind);
    }
  }
  for (from, to, weight) in &clip.arcs_out {
    if let (Some(&NodeId::Transition(nt)), Some(&NodeId::Place(np))) =
      (mapping.get(from), mapping.get(to))
    {
      let _ = app.net.add_arc_transition_to_place(nt, np, *weight);
    }
  }
  app.selection = Selection::Nodes(new_selection);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn copy_paste_duplicates_selection_and_its_internal_arc() {
    let mut app = PetriApp::new();
    let p = app.net.add_place("p");
    let t = app.net.add_transition("t");
    app
      .net
      .add_arc_place_to_transition(p, t, ArcKind::Consume(1))
      .unwrap();
    app.positions.insert(NodeId::Place(p), egui::pos2(0.0, 0.0));
    app
      .positions
      .insert(NodeId::Transition(t), egui::pos2(50.0, 0.0));
    app.selection = Selection::Nodes(HashSet::from([NodeId::Place(p), NodeId::Transition(t)]));

    copy_selection(&mut app, &egui::Context::default());
    paste_clipboard(&mut app);

    assert_eq!(app.net.place_ids().count(), 2);
    assert_eq!(app.net.transition_ids().count(), 2);
    let new_t = app.net.transition_ids().find(|&x| x != t).unwrap();
    assert_eq!(app.net.inputs(new_t).len(), 1); // the copied arc came along with it

    let Selection::Nodes(sel) = &app.selection else {
      panic!("paste should leave the new nodes selected")
    };
    assert_eq!(sel.len(), 2);
  }
}
