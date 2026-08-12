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
use crate::model::{ArcKind, PetriNet, TransitionId};

const MAGIC: [u8; 4] = *b"PCRB";
const VERSION: u8 = 1;

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
struct ProjectFile {
  places: Vec<ProjectPlace>,
  transitions: Vec<ProjectTransition>,
}

fn invalid_data(msg: impl Into<String>) -> io::Error {
  io::Error::new(io::ErrorKind::InvalidData, msg.into())
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

  let file = ProjectFile { places, transitions };
  let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&file)
    .map_err(|e| io::Error::other(e.to_string()))?;

  let mut out = Vec::with_capacity(MAGIC.len() + 1 + archived.len());
  out.extend_from_slice(&MAGIC);
  out.push(VERSION);
  out.extend_from_slice(&archived);
  std::fs::write(path, out)
}

/// Loaded state ready to drop into a fresh [`PetriApp`]: the rebuilt net, node positions,
/// transition rotations, and the `p`/`t` counters so newly added nodes keep numbering forward
/// from where the file left off.
pub struct Loaded {
  pub net: PetriNet,
  pub positions: HashMap<NodeId, egui::Pos2>,
  pub rotation: HashMap<TransitionId, f32>,
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
  let place_ids: Vec<_> = file
    .places
    .iter()
    .map(|p| {
      let id = net.add_place(p.label.clone());
      net.set_tokens(id, p.tokens);
      positions.insert(NodeId::Place(id), egui::pos2(p.x, p.y));
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

  Ok(Loaded {
    next_place_n: file.places.len(),
    next_transition_n: file.transitions.len(),
    net,
    positions,
    rotation,
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
    let _ = app
      .net
      .add_arc_place_to_transition(p1, t, ArcKind::Peek(1));
    let _ = app.net.add_arc_transition_to_place(t, p2, 2);
    app.positions.insert(NodeId::Place(p1), egui::pos2(10.0, 20.0));
    app.rotation.insert(t, 45.0);

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
  }
}
