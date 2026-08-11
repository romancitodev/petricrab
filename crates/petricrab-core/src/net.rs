use std::collections::BTreeMap;

use crate::marking::Marking;

/// This [`PetriNet`] struct represents a Petri net with **infinite** capacity places and transitions.
///
/// The **infinite** capacity is deliberated because, if we cap it, we need to add a new condition so that the transition can fire.
#[derive(Default)]
pub struct PetriNet {
  place_count: usize,
  transitions: Vec<Transition>,
}

#[expect(
  dead_code,
  reason = "We are not using this struct yet, but we will use it in the future."
)]
/// This [`PetriNetFixed`] struct represents a Petri net with **fixed** capacity places and transitions.
///
/// The capacity of each place is fixed and cannot be exceeded.
pub struct PetriNetFixed {
  place_count: usize,
  transitions: Vec<Transition>,
}

impl PetriNet {
  /// Add a place to the petri net
  pub fn add_place(&mut self) -> PlaceId {
    let id = PlaceId(self.place_count);
    self.place_count += 1;
    id
  }

  /// Add a transition to the petri net
  pub fn add_transition(&mut self, arcs: Arc) -> TransitionId {
    let id = TransitionId(self.transitions.len());
    self.transitions.push(Transition { id, arcs });
    id
  }

  /// Get the IDs of all transitions that are enabled in the given marking
  pub fn enabled_transitions(&self, marking: &Marking) -> Vec<TransitionId> {
    self
      .transitions
      .iter()
      .filter(|t| t.is_enabled(marking))
      .map(|t| t.id)
      .collect()
  }

  /// # Panics
  ///
  /// The function can panic if visited markings are too many and the stack overflows. This is a limitation of the current implementation, which uses a depth-first search approach. In practice, this should not be an issue for reasonably sized Petri nets.
  pub fn reachable_markings(
    &self,
    initial_marking: &Marking,
  ) -> BTreeMap<Marking, Vec<(TransitionId, Marking)>> {
    let mut visited = BTreeMap::new();
    let mut stack = vec![initial_marking.clone()];

    while let Some(current_marking) = stack.pop() {
      if visited.contains_key(&current_marking) {
        continue;
      }
      visited.insert(current_marking.clone(), Vec::new());

      let enabled_transitions = self.enabled_transitions(&current_marking);

      for transition_id in enabled_transitions {
        let transition = self.transition(transition_id);
        let mut new_marking = current_marking.clone();
        transition.fire(&mut new_marking);
        visited
          .get_mut(&current_marking)
          .unwrap()
          .push((transition_id, new_marking.clone()));
        stack.push(new_marking);
      }
    }

    visited
  }

  /// # Panics
  ///
  /// It panics if the tokens vector length does not match the number of places in the Petri net.
  pub fn initial_marking(&self, tokens: Vec<usize>) -> Marking {
    assert_eq!(
      tokens.len(),
      self.place_count,
      "The length of the tokens vector must match the number of places in the Petri net."
    );
    Marking::new(tokens)
  }

  /// Get a reference to a transition by its ID
  pub fn transition(&self, id: TransitionId) -> &Transition {
    &self.transitions[id.0]
  }

  /// Iterate over the IDs of every transition in the net.
  pub fn transition_ids(&self) -> impl Iterator<Item = TransitionId> + '_ {
    (0..self.transitions.len()).map(TransitionId)
  }

  /// Iterate over the IDs of every place in the net.
  pub fn place_ids(&self) -> impl Iterator<Item = PlaceId> + '_ {
    (0..self.place_count).map(PlaceId)
  }
}

#[derive(Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Weight(pub(crate) usize);
#[derive(Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaceId(pub(crate) usize);
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransitionId(usize);

/// A [`Place`] can be:
///
/// | Inputs | Output |
/// | ---    | ---    |
/// | Pre-conditions | Post-conditions |
/// | Input Data | Output Data |
/// | Input Signals | Output Signals |
/// | Resources Needed | Resources Released |
/// | Conditions | Conclusion(s) |
/// | Buffers | Buffers |
#[derive(Clone, Copy)]
pub struct Place {
  id: PlaceId,   // usize temporaly
  tokens: usize, // 1..N
}

#[derive(Default)]
/// A [`Transition`] can be:
///
/// - An event
/// - A computation step
/// - A signal processor
/// - A task or job
/// - A clause in logic
/// - A processor
pub struct Transition {
  id: TransitionId,
  arcs: Arc,
}

impl Transition {
  pub fn new() -> Self {
    Transition {
      id: TransitionId(0),
      arcs: Arc::default(),
    }
  }

  pub(crate) fn arcs(&self) -> &Arc {
    &self.arcs
  }

  pub fn from_arcs(arcs: Arc) -> Self {
    Transition {
      id: TransitionId(0),
      arcs,
    }
  }

  /// A transition is said to be enabled if:
  ///
  /// Each input place of the transition is marked with at least w(p,t) tokens, where w(p,t)
  /// is the weight of the arc from p to t.
  ///
  /// In other words, a transition is enabled if all its input places have enough tokens to satisfy the weights of the arcs leading to it.
  pub fn is_enabled(&self, marking: &Marking) -> bool {
    let inputs = self.arcs.inputs();
    inputs.iter().all(|(id, kind)| match kind {
      ArcKind::Consume(Weight(w)) => marking.tokens(*id) >= *w,
      ArcKind::Peek => marking.tokens(*id) > 0,
      ArcKind::Inhibit => marking.tokens(*id) == 0,
    })
  }

  /// Helper function to determine if a transition is a source transition (i.e., it has no input places).
  pub fn is_source(&self) -> bool {
    self.arcs.inputs().is_empty()
  }

  /// Helper function to determine if a transition is a sink transition (i.e., it has no output places).
  pub fn is_sink(&self) -> bool {
    self.arcs.output().is_empty()
  }

  /// Firing a transition involves consuming tokens from its input places and producing tokens in its output places.
  ///
  /// Note: this method actually does not take care of capacity of the [`PetriNet`], because like the docs say, the capacity is **infinite**.
  pub fn fire(&self, marking: &mut Marking) -> bool {
    if !self.is_enabled(marking) {
      return false;
    }
    self.consume_tokens(marking);
    self.forward_tokens(marking);
    true
  }

  /// Forward tokens to the output places of the transition.
  fn forward_tokens(&self, marking: &mut Marking) {
    let outputs = self.arcs.output();
    for (id, weight) in outputs {
      marking.0[id.0] += weight.0;
    }
  }

  /// Consume tokens from the input places of the transition.
  fn consume_tokens(&self, marking: &mut Marking) {
    let inputs = self.arcs.inputs();
    for (id, kind) in inputs {
      let ArcKind::Consume(weight) = kind else {
        continue;
      };
      marking.0[id.0] -= weight.0;
    }
  }
}

/// An [`ArcKind`] is a flow relation between a [`Place`] and a [`Transition`].
///
/// The same can represent a
pub enum ArcKind {
  /// -->: The token is consumed `N` times.
  Consume(Weight),
  /// <--> The token is peeked so it's not consumed.
  Peek,
  /// --o This kind of arc is special, because the [`Place`] must have no tokens for the [`Transition`] to fire.
  Inhibit,
}

#[derive(Default)]
/// flow-relation between Places -> Transition;
pub struct Arc {
  /// we can have any arc kind (like consuming or keeping) from a place to a transition
  inputs: BTreeMap<PlaceId, ArcKind>,
  /// but, from a transition to a place, it's only a consuming arc, so we must ensure the invariant.
  output: BTreeMap<PlaceId, Weight>,
}

impl Arc {
  pub fn inputs(&self) -> &BTreeMap<PlaceId, ArcKind> {
    &self.inputs
  }

  pub fn output(&self) -> &BTreeMap<PlaceId, Weight> {
    &self.output
  }

  pub fn new(inputs: BTreeMap<PlaceId, ArcKind>, output: BTreeMap<PlaceId, Weight>) -> Self {
    Arc { inputs, output }
  }

  /// Add an input arc to the transition
  pub fn add_input(&mut self, place_id: PlaceId, kind: ArcKind) -> &mut Self {
    self.inputs.insert(place_id, kind);
    self
  }

  /// Add an output arc to the transition
  pub fn add_output(&mut self, transition_id: PlaceId, weight: Weight) -> &mut Self {
    self.output.insert(transition_id, weight);
    self
  }

  /// Add an input arc to the transition without returning a reference to self
  pub fn add_input_inplace(&mut self, place_id: PlaceId, kind: ArcKind) {
    self.inputs.insert(place_id, kind);
  }

  /// Add an output arc to the transition without returning a reference to self
  pub fn add_output_inplace(&mut self, transition_id: PlaceId, weight: Weight) {
    self.output.insert(transition_id, weight);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn make_h2o() {
    let mut net = PetriNet::default();
    let h = net.add_place();
    let o = net.add_place();
    let h2o = net.add_place();

    let mut arcs = Arc::default();
    arcs
      .add_input(h, ArcKind::Consume(Weight(2)))
      .add_input(o, ArcKind::Consume(Weight(1)))
      .add_output(h2o, Weight(1));
    let reaction = net.add_transition(arcs);

    let mut marking = net.initial_marking(vec![2, 2, 0]); // 2H, 2O, 0 H2O

    println!("Initial marking: {marking:?}");

    let transition = &net.transition(reaction);
    assert!(transition.is_enabled(&marking));
    assert!(transition.fire(&mut marking));

    println!("Marking after firing: {marking:?}");

    assert_eq!(marking.tokens(h), 0);
    assert_eq!(marking.tokens(o), 1);
    assert_eq!(marking.tokens(h2o), 1);
  }

  #[test]
  fn test_reachable_markings_cycle() {
    // p1 <- t2 - p2 <-t1- p1: firing t1 then t2 returns to the initial marking,
    // so the reachability graph is a 2-node cycle.
    let mut net = PetriNet::default();
    let p1 = net.add_place();
    let p2 = net.add_place();

    let mut t1_arcs = Arc::default();
    t1_arcs
      .add_input(p1, ArcKind::Consume(Weight(1)))
      .add_output(p2, Weight(1));
    let t1 = net.add_transition(t1_arcs);

    let mut t2_arcs = Arc::default();
    t2_arcs
      .add_input(p2, ArcKind::Consume(Weight(1)))
      .add_output(p1, Weight(1));
    let t2 = net.add_transition(t2_arcs);

    let m0 = net.initial_marking(vec![1, 0]);
    let m1 = Marking::new(vec![0, 1]);

    let graph = net.reachable_markings(&m0);

    assert_eq!(graph.len(), 2, "should not loop forever on a cycle");
    assert_eq!(graph.get(&m0).unwrap(), &vec![(t1, m1.clone())]);
    assert_eq!(graph.get(&m1).unwrap(), &vec![(t2, m0.clone())]);
  }

  #[test]
  fn test_transition_not_enabled() {
    let mut net = PetriNet::default();
    let p1 = net.add_place();
    let p2 = net.add_place();
    let mut t1_arcs = Arc::default();

    t1_arcs
      .add_input(p1, ArcKind::Consume(Weight(2)))
      .add_output(p2, Weight(1));

    let t1_id = net.add_transition(t1_arcs);

    let mut marking = net.initial_marking(vec![1, 0]); // 1 token in p1, 0 in p2

    let t1 = net.transition(t1_id);
    assert!(!t1.is_enabled(&marking));
    assert!(!t1.fire(&mut marking)); // Should not fire, marking remains unchanged
  }
}
