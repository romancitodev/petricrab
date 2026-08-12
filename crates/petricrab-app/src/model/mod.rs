//! Petri net core: places, transitions, arcs, firing, and reachability exploration.

pub mod fire;

use slotmap::SlotMap;
use std::collections::BTreeMap;

slotmap::new_key_type! {
    pub struct PlaceId;
    pub struct TransitionId;
}

/// A marking is a snapshot of token counts per place.
/// Used as the state representation for firing and reachability exploration.
pub type Marking = BTreeMap<PlaceId, u32>;

/// Kind of an input arc (place -> transition), carrying its weight. Output arcs
/// (transition -> place) always produce tokens, so they just carry a plain weight.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ArcKind {
  /// `-->` normal arc: requires `tokens >= weight`, consumes `weight` tokens on fire.
  Consume(u32),
  /// `<-->` test/read arc: requires `tokens >= weight`, does not consume tokens.
  Peek(u32),
  /// `--o` inhibitor arc: requires `tokens < weight`, does not consume tokens.
  Inhibit(u32),
}

impl ArcKind {
  pub fn weight(self) -> u32 {
    let (ArcKind::Consume(w) | ArcKind::Peek(w) | ArcKind::Inhibit(w)) = self;
    w
  }
}

struct PlaceData {
  label: String,
  tokens: u32,
}

struct TransitionData {
  label: String,
  inputs: Vec<(PlaceId, ArcKind)>,
  /// (place, weight) for arcs transition -> place
  outputs: Vec<(PlaceId, u32)>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ArcError {
  UnknownPlace,
  UnknownTransition,
  ArcAlreadyExists,
}

#[derive(Default)]
pub struct PetriNet {
  places: SlotMap<PlaceId, PlaceData>,
  transitions: SlotMap<TransitionId, TransitionData>,
}

impl PetriNet {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn add_place(&mut self, label: impl Into<String>) -> PlaceId {
    self.places.insert(PlaceData {
      label: label.into(),
      tokens: 0,
    })
  }

  pub fn add_transition(&mut self, label: impl Into<String>) -> TransitionId {
    self.transitions.insert(TransitionData {
      label: label.into(),
      inputs: Vec::new(),
      outputs: Vec::new(),
    })
  }

  pub fn add_arc_place_to_transition(
    &mut self,
    p: PlaceId,
    t: TransitionId,
    kind: ArcKind,
  ) -> Result<(), ArcError> {
    if !self.places.contains_key(p) {
      return Err(ArcError::UnknownPlace);
    }
    let transition = self
      .transitions
      .get_mut(t)
      .ok_or(ArcError::UnknownTransition)?;
    if transition.inputs.iter().any(|(place, _)| *place == p) {
      return Err(ArcError::ArcAlreadyExists);
    }
    transition.inputs.push((p, kind));
    Ok(())
  }

  pub fn add_arc_transition_to_place(
    &mut self,
    t: TransitionId,
    p: PlaceId,
    weight: u32,
  ) -> Result<(), ArcError> {
    if !self.places.contains_key(p) {
      return Err(ArcError::UnknownPlace);
    }
    let transition = self
      .transitions
      .get_mut(t)
      .ok_or(ArcError::UnknownTransition)?;
    if transition.outputs.iter().any(|(place, _)| *place == p) {
      return Err(ArcError::ArcAlreadyExists);
    }
    transition.outputs.push((p, weight));
    Ok(())
  }

  pub fn remove_place(&mut self, p: PlaceId) {
    self.places.remove(p);
    for transition in self.transitions.values_mut() {
      transition.inputs.retain(|(place, _)| *place != p);
      transition.outputs.retain(|(place, _)| *place != p);
    }
  }

  pub fn remove_transition(&mut self, t: TransitionId) {
    self.transitions.remove(t);
  }

  pub fn remove_arc_place_to_transition(&mut self, p: PlaceId, t: TransitionId) {
    if let Some(transition) = self.transitions.get_mut(t) {
      transition.inputs.retain(|(place, _)| *place != p);
    }
  }

  pub fn remove_arc_transition_to_place(&mut self, t: TransitionId, p: PlaceId) {
    if let Some(transition) = self.transitions.get_mut(t) {
      transition.outputs.retain(|(place, _)| *place != p);
    }
  }

  pub fn tokens(&self, p: PlaceId) -> u32 {
    self.places.get(p).map_or(0, |place| place.tokens)
  }

  pub fn set_tokens(&mut self, p: PlaceId, n: u32) {
    if let Some(place) = self.places.get_mut(p) {
      place.tokens = n;
    }
  }

  pub fn marking(&self) -> Marking {
    self
      .places
      .iter()
      .map(|(id, place)| (id, place.tokens))
      .collect()
  }

  pub fn set_marking(&mut self, m: &Marking) {
    for (id, place) in self.places.iter_mut() {
      place.tokens = *m.get(&id).unwrap_or(&0);
    }
  }

  pub fn place_label(&self, id: PlaceId) -> &str {
    self
      .places
      .get(id)
      .map_or("<removed>", |place| place.label.as_str())
  }

  pub fn transition_label(&self, id: TransitionId) -> &str {
    self
      .transitions
      .get(id)
      .map_or("<removed>", |transition| transition.label.as_str())
  }

  pub fn place_label_mut(&mut self, id: PlaceId) -> Option<&mut String> {
    self.places.get_mut(id).map(|place| &mut place.label)
  }

  pub fn transition_label_mut(&mut self, id: TransitionId) -> Option<&mut String> {
    self
      .transitions
      .get_mut(id)
      .map(|transition| &mut transition.label)
  }

  pub fn place_ids(&self) -> impl Iterator<Item = PlaceId> + '_ {
    self.places.keys()
  }

  pub fn transition_ids(&self) -> impl Iterator<Item = TransitionId> + '_ {
    self.transitions.keys()
  }

  /// Input arcs place -> transition feeding `t`, as (place, kind).
  pub fn inputs(&self, t: TransitionId) -> &[(PlaceId, ArcKind)] {
    self
      .transitions
      .get(t)
      .map_or(&[], |transition| transition.inputs.as_slice())
  }

  /// Output arcs transition -> place produced by `t`, as (place, weight).
  pub fn outputs(&self, t: TransitionId) -> &[(PlaceId, u32)] {
    self
      .transitions
      .get(t)
      .map_or(&[], |transition| transition.outputs.as_slice())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn add_place_and_transition_with_arcs_tracks_tokens_and_marking() {
    let mut net = PetriNet::new();
    let p1 = net.add_place("p1");
    let p2 = net.add_place("p2");
    let t = net.add_transition("t");

    net
      .add_arc_place_to_transition(p1, t, ArcKind::Consume(2))
      .unwrap();
    net.add_arc_transition_to_place(t, p2, 1).unwrap();

    net.set_tokens(p1, 5);
    assert_eq!(net.tokens(p1), 5);
    assert_eq!(net.tokens(p2), 0);

    let marking = net.marking();
    assert_eq!(marking.get(&p1), Some(&5));
    assert_eq!(marking.get(&p2), Some(&0));
  }

  #[test]
  fn duplicate_arc_is_rejected() {
    let mut net = PetriNet::new();
    let p = net.add_place("p");
    let t = net.add_transition("t");

    net
      .add_arc_place_to_transition(p, t, ArcKind::Consume(1))
      .unwrap();
    let result = net.add_arc_place_to_transition(p, t, ArcKind::Consume(1));
    assert_eq!(result, Err(ArcError::ArcAlreadyExists));
  }

  #[test]
  fn remove_place_clears_dangling_arcs() {
    let mut net = PetriNet::new();
    let p = net.add_place("p");
    let t = net.add_transition("t");
    net
      .add_arc_place_to_transition(p, t, ArcKind::Consume(1))
      .unwrap();

    net.remove_place(p);

    assert!(net.inputs(t).is_empty());
  }

  #[test]
  fn remove_arc_deletes_only_matching_arc() {
    let mut net = PetriNet::new();
    let p1 = net.add_place("p1");
    let p2 = net.add_place("p2");
    let t = net.add_transition("t");
    net
      .add_arc_place_to_transition(p1, t, ArcKind::Consume(1))
      .unwrap();
    net
      .add_arc_place_to_transition(p2, t, ArcKind::Consume(1))
      .unwrap();
    net.add_arc_transition_to_place(t, p1, 1).unwrap();

    net.remove_arc_place_to_transition(p1, t);
    net.remove_arc_transition_to_place(t, p2); // no-op: that arc never existed

    assert_eq!(net.inputs(t), &[(p2, ArcKind::Consume(1))]);
    assert_eq!(net.outputs(t), &[(p1, 1)]);
  }

  #[test]
  fn set_marking_restores_token_counts() {
    let mut net = PetriNet::new();
    let p1 = net.add_place("p1");
    let p2 = net.add_place("p2");

    let mut m = Marking::new();
    m.insert(p1, 3);
    m.insert(p2, 7);
    net.set_marking(&m);

    assert_eq!(net.tokens(p1), 3);
    assert_eq!(net.tokens(p2), 7);
  }
}
