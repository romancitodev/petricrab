//! Binary project file format: a snapshot of the net (labels, arcs, tokens) plus the editor
//! state needed to redraw it (positions, transition rotation). Saved as rkyv-archived bytes
//! behind a small magic/version header, not a human-readable format — nothing here is meant to
//! be inspected outside petricrab.
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::Path;

use eframe::egui;
use rkyv::{Archive, Deserialize, Serialize};

use crate::app::{NodeId, PetriApp};
use crate::model::{ArcKind, PetriNet, PlaceId, TransitionId};

const MAGIC: [u8; 4] = *b"PCRB";
const VERSION: u8 = 3;

#[derive(Archive, Serialize, Deserialize, Clone, Copy)]
enum ProjectArcKind {
  Consume,
  Peek,
  Inhibit,
}

impl ProjectArcKind {
  fn from_kind(kind: ArcKind) -> (Self, u32) {
    match kind {
      ArcKind::Consume(w) => (Self::Consume, w),
      ArcKind::Peek(w) => (Self::Peek, w),
      ArcKind::Inhibit(w) => (Self::Inhibit, w),
    }
  }

  fn to_kind(self, weight: u32) -> ArcKind {
    match self {
      Self::Consume => ArcKind::Consume(weight),
      Self::Peek => ArcKind::Peek(weight),
      Self::Inhibit => ArcKind::Inhibit(weight),
    }
  }
}

#[derive(Archive, Serialize, Deserialize)]
struct ProjectPlace {
  label: String,
  tokens: u32,
  x: f32,
  y: f32,
  /// `None` = no custom color, use the theme default.
  color: Option<(u8, u8, u8, u8)>,
}

#[derive(Archive, Serialize, Deserialize)]
struct ProjectTransition {
  label: String,
  rotation: f32,
  x: f32,
  y: f32,
  /// (index into `ProjectFile::places`, arc kind, weight)
  inputs: Vec<(u32, ProjectArcKind, u32)>,
  /// (index into `ProjectFile::places`, weight)
  outputs: Vec<(u32, u32)>,
}

#[derive(Archive, Serialize, Deserialize)]
struct ProjectNote {
  text: String,
  x: f32,
  y: f32,
  w: f32,
  h: f32,
  /// `None` = no custom color, use the theme default.
  color: Option<(u8, u8, u8, u8)>,
}

#[derive(Archive, Serialize, Deserialize)]
struct ProjectFile {
  places: Vec<ProjectPlace>,
  transitions: Vec<ProjectTransition>,
  notes: Vec<ProjectNote>,
}

fn invalid_data(msg: impl Into<String>) -> io::Error {
  io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Highest `N` among labels of the form `<prefix>N` (e.g. "p3", "t12") — 0 if none match.
/// `next_place_n`/`next_transition_n` need to resume from here, not from a plain item count:
/// a file with labels "t1, t2, t4" (a "t3" deleted before saving) has 3 transitions but the
/// next auto-generated label must skip past 4, or it collides with the surviving "t4".
fn max_numeric_suffix<'a>(labels: impl Iterator<Item = &'a str>, prefix: char) -> usize {
  labels
    .filter_map(|label| label.strip_prefix(prefix)?.parse::<usize>().ok())
    .max()
    .unwrap_or(0)
}

pub fn save(app: &PetriApp, path: &Path) -> io::Result<()> {
  let place_ids: Vec<_> = app.net.place_ids().collect();
  let place_index: HashMap<_, u32> = place_ids
    .iter()
    .enumerate()
    .map(|(i, &id)| (id, i as u32))
    .collect();

  let places = place_ids
    .iter()
    .map(|&p| {
      let pos = app
        .positions
        .get(&NodeId::Place(p))
        .copied()
        .unwrap_or_default();
      ProjectPlace {
        label: app.net.place_label(p).to_string(),
        tokens: app.net.tokens(p),
        x: pos.x,
        y: pos.y,
        color: app.colors.get(&p).map(|c| (c.r(), c.g(), c.b(), c.a())),
      }
    })
    .collect();

  let transitions = app
    .net
    .transition_ids()
    .map(|t| {
      let pos = app
        .positions
        .get(&NodeId::Transition(t))
        .copied()
        .unwrap_or_default();
      let inputs = app
        .net
        .inputs(t)
        .iter()
        .map(|&(p, kind)| {
          let (tag, weight) = ProjectArcKind::from_kind(kind);
          (place_index[&p], tag, weight)
        })
        .collect();
      let outputs = app
        .net
        .outputs(t)
        .iter()
        .map(|&(p, w)| (place_index[&p], w))
        .collect();
      ProjectTransition {
        label: app.net.transition_label(t).to_string(),
        rotation: app.rotation.get(&t).copied().unwrap_or(0.0),
        x: pos.x,
        y: pos.y,
        inputs,
        outputs,
      }
    })
    .collect();

  let notes = app
    .notes
    .values()
    .map(|n| ProjectNote {
      text: n.text.clone(),
      x: n.pos.x,
      y: n.pos.y,
      w: n.size.x,
      h: n.size.y,
      color: n.color.map(|c| (c.r(), c.g(), c.b(), c.a())),
    })
    .collect();

  let file = ProjectFile {
    places,
    transitions,
    notes,
  };
  let archived =
    rkyv::to_bytes::<rkyv::rancor::Error>(&file).map_err(|e| io::Error::other(e.to_string()))?;

  let mut out = Vec::with_capacity(MAGIC.len() + 1 + archived.len());
  out.extend_from_slice(&MAGIC);
  out.push(VERSION);
  out.extend_from_slice(&archived);
  std::fs::write(path, out)
}

/// Loaded state ready to drop into a fresh [`PetriApp`]: the rebuilt net, node positions,
/// transition rotations, custom place colors, and the `p`/`t` counters so newly added nodes
/// keep numbering forward from where the file left off.
pub struct Loaded {
  pub net: PetriNet,
  pub positions: HashMap<NodeId, egui::Pos2>,
  pub rotation: HashMap<TransitionId, f32>,
  pub colors: HashMap<PlaceId, egui::Color32>,
  pub notes: slotmap::SlotMap<crate::app::NoteId, crate::app::NoteData>,
  pub next_place_n: usize,
  pub next_transition_n: usize,
}

pub fn load(path: &Path) -> io::Result<Loaded> {
  let mut reader = std::fs::File::open(path)?;

  let mut header = [0u8; MAGIC.len() + 1];
  reader
    .read_exact(&mut header)
    .map_err(|_| invalid_data("no es un archivo de proyecto de petricrab"))?;
  let (magic, version) = header.split_at(MAGIC.len());
  if magic != MAGIC {
    return Err(invalid_data("no es un archivo de proyecto de petricrab"));
  }
  if version[0] != VERSION {
    return Err(invalid_data(format!(
      "versión de formato no soportada: {}",
      version[0]
    )));
  }

  // rkyv accesses the archive in place, so the byte buffer needs to start at an aligned
  // address — reading straight into a plain `Vec<u8>` (or slicing one after the header) doesn't
  // guarantee that. `AlignedVec` does.
  let mut bytes = rkyv::util::AlignedVec::<16>::new();
  bytes
    .extend_from_reader(&mut reader)
    .map_err(|e| invalid_data(e.to_string()))?;

  let file = rkyv::from_bytes::<ProjectFile, rkyv::rancor::Error>(&bytes)
    .map_err(|e| invalid_data(e.to_string()))?;

  let mut net = PetriNet::new();
  let mut positions = HashMap::new();
  let mut colors = HashMap::new();
  let place_ids: Vec<_> = file
    .places
    .iter()
    .map(|p| {
      let id = net.add_place(p.label.clone());
      net.set_tokens(id, p.tokens);
      positions.insert(NodeId::Place(id), egui::pos2(p.x, p.y));
      if let Some((r, g, b, a)) = p.color {
        colors.insert(id, egui::Color32::from_rgba_premultiplied(r, g, b, a));
      }
      id
    })
    .collect();

  let mut rotation = HashMap::new();
  for t in &file.transitions {
    let id = net.add_transition(t.label.clone());
    positions.insert(NodeId::Transition(id), egui::pos2(t.x, t.y));
    if t.rotation != 0.0 {
      rotation.insert(id, t.rotation);
    }
    for &(place_idx, kind, weight) in &t.inputs {
      if let Some(&p) = place_ids.get(place_idx as usize) {
        let _ = net.add_arc_place_to_transition(p, id, kind.to_kind(weight));
      }
    }
    for &(place_idx, weight) in &t.outputs {
      if let Some(&p) = place_ids.get(place_idx as usize) {
        let _ = net.add_arc_transition_to_place(id, p, weight);
      }
    }
  }

  let mut notes = slotmap::SlotMap::default();
  for n in &file.notes {
    notes.insert(crate::app::NoteData {
      pos: egui::pos2(n.x, n.y),
      size: egui::vec2(n.w, n.h),
      text: n.text.clone(),
      color: n
        .color
        .map(|(r, g, b, a)| egui::Color32::from_rgba_premultiplied(r, g, b, a)),
    });
  }

  Ok(Loaded {
    next_place_n: max_numeric_suffix(file.places.iter().map(|p| p.label.as_str()), 'p'),
    next_transition_n: max_numeric_suffix(file.transitions.iter().map(|t| t.label.as_str()), 't'),
    net,
    positions,
    rotation,
    colors,
    notes,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn save_then_load_roundtrips_places_transitions_and_arcs() {
    let mut app = PetriApp::new();
    let p1 = app.net.add_place("p1");
    let p2 = app.net.add_place("p2");
    let t = app.net.add_transition("t1");
    app.net.set_tokens(p1, 3);
    let _ = app.net.add_arc_place_to_transition(p1, t, ArcKind::Peek(1));
    let _ = app.net.add_arc_transition_to_place(t, p2, 2);
    app
      .positions
      .insert(NodeId::Place(p1), egui::pos2(10.0, 20.0));
    app.rotation.insert(t, 45.0);
    app.colors.insert(p1, egui::Color32::from_rgb(200, 60, 30));
    app.notes.insert(crate::app::NoteData {
      pos: egui::pos2(5.0, 6.0),
      size: egui::vec2(180.0, 100.0),
      text: "leyenda".to_string(),
      color: Some(egui::Color32::from_rgb(80, 120, 200)),
    });

    let dir = std::env::temp_dir();
    let path = dir.join(format!("petricrab-test-{}.gpn", std::process::id()));
    save(&app, &path).unwrap();
    let loaded = load(&path).unwrap();
    std::fs::remove_file(&path).unwrap();

    assert_eq!(loaded.net.place_ids().count(), 2);
    assert_eq!(loaded.net.transition_ids().count(), 1);
    let loaded_t = loaded.net.transition_ids().next().unwrap();
    assert_eq!(loaded.net.transition_label(loaded_t), "t1");
    assert_eq!(loaded.rotation.get(&loaded_t), Some(&45.0));

    let loaded_p1 = loaded
      .net
      .place_ids()
      .find(|&p| loaded.net.place_label(p) == "p1")
      .unwrap();
    assert_eq!(loaded.net.tokens(loaded_p1), 3);
    assert_eq!(
      loaded.positions.get(&NodeId::Place(loaded_p1)),
      Some(&egui::pos2(10.0, 20.0))
    );
    assert_eq!(loaded.net.inputs(loaded_t).len(), 1);
    assert_eq!(loaded.net.outputs(loaded_t).len(), 1);
    assert_eq!(
      loaded.colors.get(&loaded_p1),
      Some(&egui::Color32::from_rgb(200, 60, 30))
    );
    assert_eq!(loaded.notes.len(), 1);
    let loaded_note = loaded.notes.values().next().unwrap();
    assert_eq!(loaded_note.text, "leyenda");
    assert_eq!(loaded_note.pos, egui::pos2(5.0, 6.0));
    assert_eq!(
      loaded_note.color,
      Some(egui::Color32::from_rgb(80, 120, 200))
    );
  }

  #[test]
  fn load_resumes_numbering_past_gaps_left_by_deleted_nodes() {
    // Simulates a file saved after "p3"/"t3" were deleted: surviving labels are p1, p2, p4
    // and t1, t2, t4 — non-contiguous, so a plain item count (3) would hand out "p4"/"t4"
    // again on the next add, colliding with the ones already there.
    let mut app = PetriApp::new();
    for label in ["p1", "p2", "p4"] {
      app.net.add_place(label);
    }
    for label in ["t1", "t2", "t4"] {
      app.net.add_transition(label);
    }

    let dir = std::env::temp_dir();
    let path = dir.join(format!("petricrab-test-gap-{}.gpn", std::process::id()));
    save(&app, &path).unwrap();
    let loaded = load(&path).unwrap();
    std::fs::remove_file(&path).unwrap();

    assert_eq!(loaded.next_place_n, 4);
    assert_eq!(loaded.next_transition_n, 4);
  }
}
