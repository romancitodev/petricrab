use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::coverability::{ExtendedMarking, coverability_graph};
use crate::marking::Marking;
use crate::net::{PetriNet, TransitionId};

/// Every dead node reachable from `initial` in `graph`, paired with the shortest firing
/// sequence that reaches it. Shared core behind [`deadlocks`] and [`deadlocks_covering`].
fn dead_ends_from<M: Ord + Clone>(
  graph: &BTreeMap<M, Vec<(TransitionId, M)>>,
  initial: &M,
) -> BTreeMap<M, Vec<TransitionId>> {
  let root = graph.get_key_value(initial).unwrap().0;

  let mut came_from: BTreeMap<&M, (TransitionId, &M)> = BTreeMap::new();
  let mut seen = BTreeSet::from([root]);
  let mut queue = VecDeque::from([root]);

  while let Some(current) = queue.pop_front() {
    for (transition, next) in &graph[current] {
      if seen.insert(next) {
        came_from.insert(next, (*transition, current));
        queue.push_back(next);
      }
    }
  }

  graph
    .iter()
    .filter(|(_, edges)| edges.is_empty())
    .map(|(dead, _)| {
      let mut path = Vec::new();
      let mut cursor = dead;
      while let Some((transition, prev)) = came_from.get(cursor) {
        path.push(*transition);
        cursor = *prev;
      }
      path.reverse();
      (dead.clone(), path)
    })
    .collect()
}

/// Every dead marking reachable from `initial_marking`, paired with the shortest firing
/// sequence that reaches it.
///
/// # Panics
///
/// Same caveat as [`PetriNet::reachable_markings`]: never returns if the net is unbounded. Use
/// [`deadlocks_covering`] instead when that's a possibility.
pub fn deadlocks(net: &PetriNet, initial_marking: &Marking) -> BTreeMap<Marking, Vec<TransitionId>> {
  let graph = net.reachable_markings(initial_marking);
  dead_ends_from(&graph, initial_marking)
}

/// Same as [`deadlocks`], over the Karp-Miller coverability graph instead, so it terminates on
/// unbounded nets too. Returns one witness path per dead node, without the marking itself (a
/// covering node can carry `Omega`, which doesn't map back to a single concrete marking).
///
/// Sound for nets without `Inhibit` arcs on an unbounded place; can false-positive otherwise,
/// same caveat as [`crate::liveness::liveness_report_covering`].
pub fn deadlocks_covering(net: &PetriNet, initial_marking: &Marking) -> Vec<Vec<TransitionId>> {
  let graph = coverability_graph(net, initial_marking);
  let initial: ExtendedMarking = initial_marking.into();
  dead_ends_from(&graph, &initial).into_values().collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::{Arc, ArcKind, Weight};

  #[test]
  fn test_deadlocks_finds_dead_end_with_witness_path() {
    let mut net = PetriNet::default();
    let p1 = net.add_place();
    let p2 = net.add_place();
    let p3 = net.add_place();

    let mut t1_arcs = Arc::default();
    t1_arcs
      .add_input(p1, ArcKind::Consume(Weight(1)))
      .add_output(p2, Weight(1));
    let t1 = net.add_transition(t1_arcs);

    let mut t2_arcs = Arc::default();
    t2_arcs
      .add_input(p2, ArcKind::Consume(Weight(1)))
      .add_output(p3, Weight(1));
    let t2 = net.add_transition(t2_arcs);

    let m0 = net.initial_marking(vec![1, 0, 0]);
    let dead_end = Marking::new(vec![0, 0, 1]);

    let found = deadlocks(&net, &m0);

    assert_eq!(found.len(), 1);
    assert_eq!(found[&dead_end], vec![t1, t2]);
  }

  #[test]
  fn test_deadlocks_empty_on_a_cycle() {
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

    assert!(deadlocks(&net, &m0).is_empty());
  }

  #[test]
  fn test_deadlocks_covering_finds_deadlocks_past_unbounded_growth() {
    let mut net = PetriNet::default();
    let p1 = net.add_place();
    let p2 = net.add_place();

    let mut t_grow_arcs = Arc::default();
    t_grow_arcs
      .add_input(p2, ArcKind::Consume(Weight(1)))
      .add_output(p2, Weight(1))
      .add_output(p1, Weight(1));
    let t_grow = net.add_transition(t_grow_arcs);

    let mut t_drain_arcs = Arc::default();
    t_drain_arcs.add_input(p2, ArcKind::Consume(Weight(1)));
    let t_drain = net.add_transition(t_drain_arcs);

    let m0 = net.initial_marking(vec![0, 1]);

    let found = deadlocks_covering(&net, &m0);

    assert_eq!(found.len(), 2);
    assert!(found.contains(&vec![t_drain]));
    assert!(found.contains(&vec![t_grow, t_drain]));
  }
}
