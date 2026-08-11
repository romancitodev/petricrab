use std::collections::{BTreeMap, BTreeSet};

use crate::marking::Marking;
use crate::net::TransitionId;

/// Liveness levels for a transition, following Murata (1989).
///
/// The names describe the semantics, but each variant's documentation
/// preserves the formal definition. The name alone is not enough to capture
/// the distinction between L2 and L3, which is a change in quantifier
///
/// **(`∀k ∃sequence` vs. `∃sequence ∀k`)**, not a matter of degree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Liveness {
  /// Level 0: The transition can never be fired in any firing sequence from the initial marking.
  Dead,
  /// Level 1: At least one transition can be fired at least once in some firing sequence from the initial marking.
  PotentiallyFirable,
  /// Level 2: for every natural number _k_, there exists a firing sequence in which the transition can be fired at least _k_ times.
  ///
  /// _(the sequence could be different for each k)_
  ArbitrarilyRepeatable,
  /// Level 3: Exists at least one firing sequence in which the transition can be fired infinitely often.
  RepeatableForever,
  /// Level 4: The transition is L1 for every Marking in the reachability graph.
  Total,
}

impl Liveness {
  fn as_lk(self) -> u8 {
    self as u8
  }
}

/// A report of the liveness level of a transition, including the level, the maximum number of times it can be fired (if applicable), and an example firing sequence.
pub struct LivenessReport {
  pub level: Liveness,
  pub k: Option<usize>,
  pub example: Vec<TransitionId>,
}

/// Check if a marking can reach another marking in the reachability graph
fn can_reach(
  graph: &BTreeMap<Marking, Vec<(TransitionId, Marking)>>,
  from: &Marking,
  to: &Marking,
) -> bool {
  if from == to {
    return true;
  }

  let mut visited = BTreeSet::new();
  let mut stack = vec![from.clone()];

  while let Some(current) = stack.pop() {
    if !visited.insert(current.clone()) {
      continue;
    }
    let Some(transitions) = graph.get(&current) else {
      continue;
    };
    for (_, next_marking) in transitions {
      if next_marking == to {
        return true;
      }
      stack.push(next_marking.clone());
    }
  }

  false
}

fn transitions_in_cycle(
  graph: &BTreeMap<Marking, Vec<(TransitionId, Marking)>>,
) -> BTreeSet<TransitionId> {
  graph
    .iter()
    .flat_map(|(from, edges)| {
      edges
        .iter()
        .filter(move |(_, to)| can_reach(graph, to, from))
        .map(|(transition, _)| *transition)
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::{Arc, ArcKind, PetriNet, Weight};

  #[test]
  fn test_can_reach_cycle() {
    // p1 -t1-> p2 -t2-> p1: a 2-node cycle in the reachability graph.
    let mut net = PetriNet::default();
    let p1 = net.add_place();
    let p2 = net.add_place();

    let mut t1_arcs = Arc::default();
    t1_arcs
      .add_input(p1, ArcKind::Consume(Weight(1)))
      .add_output(p2, Weight(1));
    net.add_transition(t1_arcs);

    let mut t2_arcs = Arc::default();
    t2_arcs
      .add_input(p2, ArcKind::Consume(Weight(1)))
      .add_output(p1, Weight(1));
    net.add_transition(t2_arcs);

    let m0 = net.initial_marking(vec![1, 0]);
    let m1 = Marking::new(vec![0, 1]);
    let unreachable = Marking::new(vec![5, 5]);
    let graph = net.reachable_markings(&m0);

    assert!(can_reach(&graph, &m1, &m0), "m1 should reach m0 via t2");
    assert!(
      can_reach(&graph, &m0, &m0),
      "self-loop back through the cycle"
    );
    assert!(
      !can_reach(&graph, &m0, &unreachable),
      "must terminate, not stack overflow, when the target is unreachable"
    );
  }

  #[test]
  fn test_transitions_in_cycle_detects_cycle() {
    // p1 -t1-> p2 -t2-> p1: both transitions are on the cycle.
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
    let graph = net.reachable_markings(&m0);

    let in_cycle = transitions_in_cycle(&graph);

    assert_eq!(in_cycle, BTreeSet::from([t1, t2]));
  }

  #[test]
  fn test_transitions_in_cycle_excludes_dead_end() {
    // p1 -t1-> p2, no way back: t1 fires once and the graph dead-ends.
    let mut net = PetriNet::default();
    let p1 = net.add_place();
    let p2 = net.add_place();

    let mut t1_arcs = Arc::default();
    t1_arcs
      .add_input(p1, ArcKind::Consume(Weight(1)))
      .add_output(p2, Weight(1));
    net.add_transition(t1_arcs);

    let m0 = net.initial_marking(vec![1, 0]);
    let graph = net.reachable_markings(&m0);

    let in_cycle = transitions_in_cycle(&graph);

    assert!(in_cycle.is_empty(), "a dead-end transition is not on a cycle");
  }
}
