use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::marking::Marking;
use crate::net::{PetriNet, TransitionId};

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
pub(crate) fn can_reach(
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

/// Whether `transition` can eventually fire starting from `from` i.e. `from`
/// itself, or some marking reachable from it, has `transition` as an outgoing edge.
fn fires_from(
  graph: &BTreeMap<Marking, Vec<(TransitionId, Marking)>>,
  from: &Marking,
  transition: TransitionId,
) -> bool {
  let mut visited = BTreeSet::new();
  let mut stack = vec![from.clone()];

  while let Some(current) = stack.pop() {
    if !visited.insert(current.clone()) {
      continue;
    }
    let Some(edges) = graph.get(&current) else {
      continue;
    };
    for (edge_transition, next_marking) in edges {
      if *edge_transition == transition {
        return true;
      }
      stack.push(next_marking.clone());
    }
  }

  false
}

/// Shortest firing sequence from `from` that fires `target` at least once, or `None` if
/// `target` never fires from `from` (i.e. it's dead there). Breadth-first with parent pointers
/// keyed by marking, so it's the minimum-length witness rather than whatever path a DFS
/// happens to find first.
fn firing_path_to(
  graph: &BTreeMap<Marking, Vec<(TransitionId, Marking)>>,
  from: &Marking,
  target: TransitionId,
) -> Option<Vec<TransitionId>> {
  let mut visited = BTreeSet::from([from.clone()]);
  let mut queue = VecDeque::from([from.clone()]);
  let mut came_from: BTreeMap<Marking, (TransitionId, Marking)> = BTreeMap::new();

  while let Some(current) = queue.pop_front() {
    let Some(edges) = graph.get(&current) else {
      continue;
    };
    for (transition, next) in edges {
      if *transition == target {
        let mut path = Vec::new();
        let mut cursor = current.clone();
        while let Some((t, prev)) = came_from.get(&cursor) {
          path.push(*t);
          cursor = prev.clone();
        }
        path.reverse();
        path.push(target);
        return Some(path);
      }
      if visited.insert(next.clone()) {
        came_from.insert(next.clone(), (*transition, current.clone()));
        queue.push_back(next.clone());
      }
    }
  }

  None
}

/// Classify the liveness of `transition` over the reachability graph rooted at
/// `initial_marking`.
pub fn liveness_of(
  graph: &BTreeMap<Marking, Vec<(TransitionId, Marking)>>,
  initial_marking: &Marking,
  transition: TransitionId,
) -> LivenessReport {
  if !fires_from(graph, initial_marking, transition) {
    return LivenessReport {
      level: Liveness::Dead,
      k: Some(0),
      example: Vec::new(),
    };
  }

  // Total (L4) implies RepeatableForever (L3) implies ArbitrarilyRepeatable (L2)
  // implies PotentiallyFirable (L1), so check strongest-first. On our finite
  // reachability graph L2 and L3 always coincide (Murata), so we never report
  // ArbitrarilyRepeatable on its own — see the doc comment on `Liveness`.
  let level = if graph
    .keys()
    .all(|marking| fires_from(graph, marking, transition))
  {
    Liveness::Total
  } else if transitions_in_cycle(graph).contains(&transition) {
    Liveness::RepeatableForever
  } else {
    Liveness::PotentiallyFirable
  };

  let example = firing_path_to(graph, initial_marking, transition).unwrap_or_default();

  LivenessReport {
    level,
    k: None,
    example,
  }
}

/// Every transition reachable from `from` — i.e. every `TransitionId` labeling
/// an edge on some path starting at `from` (including `from`'s own outgoing edges).
fn reachable_transitions_from(
  graph: &BTreeMap<Marking, Vec<(TransitionId, Marking)>>,
  from: &Marking,
) -> BTreeSet<TransitionId> {
  let mut found = BTreeSet::new();
  let mut visited = BTreeSet::new();
  let mut stack = vec![from.clone()];

  while let Some(current) = stack.pop() {
    if !visited.insert(current.clone()) {
      continue;
    }
    let Some(edges) = graph.get(&current) else {
      continue;
    };
    for (transition, next_marking) in edges {
      found.insert(*transition);
      stack.push(next_marking.clone());
    }
  }

  found
}

/// Classify the liveness of every transition in `net`, over the reachability
/// graph rooted at `initial_marking`.
///
/// Unlike calling [`liveness_of`] once per transition, this computes the
/// reachability graph, the cycle-membership set, and each marking's reachable
/// transitions exactly once, then classifies every transition off those
/// precomputed sets in O(1) each.
pub fn liveness_report(
  net: &PetriNet,
  initial_marking: &Marking,
) -> BTreeMap<TransitionId, LivenessReport> {
  let graph = net.reachable_markings(initial_marking);
  let in_cycle = transitions_in_cycle(&graph);

  let reachable_per_marking: BTreeMap<&Marking, BTreeSet<TransitionId>> = graph
    .keys()
    .map(|marking| (marking, reachable_transitions_from(&graph, marking)))
    .collect();

  let total: BTreeSet<TransitionId> = reachable_per_marking
    .values()
    .cloned()
    .reduce(|acc, set| acc.intersection(&set).copied().collect())
    .unwrap_or_default();

  let fires_from_initial = reachable_per_marking
    .get(initial_marking)
    .cloned()
    .unwrap_or_default();

  net
    .transition_ids()
    .map(|transition| {
      let level = if !fires_from_initial.contains(&transition) {
        Liveness::Dead
      } else if total.contains(&transition) {
        Liveness::Total
      } else if in_cycle.contains(&transition) {
        Liveness::RepeatableForever
      } else {
        Liveness::PotentiallyFirable
      };
      let k = (level == Liveness::Dead).then_some(0);
      let example = firing_path_to(&graph, initial_marking, transition).unwrap_or_default();

      (transition, LivenessReport { level, k, example })
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::{Arc, ArcKind, Weight};

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

    assert!(
      in_cycle.is_empty(),
      "a dead-end transition is not on a cycle"
    );
  }

  #[test]
  fn test_liveness_of_dead() {
    // p1 starts empty, so t1 is never enabled anywhere in the graph.
    let mut net = PetriNet::default();
    let p1 = net.add_place();

    let mut t1_arcs = Arc::default();
    t1_arcs.add_input(p1, ArcKind::Consume(Weight(1)));
    let t1 = net.add_transition(t1_arcs);

    let m0 = net.initial_marking(vec![0]);
    let graph = net.reachable_markings(&m0);

    let report = liveness_of(&graph, &m0, t1);

    assert_eq!(report.level, Liveness::Dead);
    assert_eq!(report.k, Some(0));
  }

  #[test]
  fn test_liveness_of_potentially_firable_dead_end() {
    // p1 -t1-> p2, no way back: fires once, but not from every marking (p2 has
    // no outgoing edges), so it's L1 but neither cycling nor Total.
    let mut net = PetriNet::default();
    let p1 = net.add_place();
    let p2 = net.add_place();

    let mut t1_arcs = Arc::default();
    t1_arcs
      .add_input(p1, ArcKind::Consume(Weight(1)))
      .add_output(p2, Weight(1));
    let t1 = net.add_transition(t1_arcs);

    let m0 = net.initial_marking(vec![1, 0]);
    let graph = net.reachable_markings(&m0);

    let report = liveness_of(&graph, &m0, t1);

    assert_eq!(report.level, Liveness::PotentiallyFirable);
    assert_eq!(
      report.example,
      vec![t1],
      "t1 fires directly from m0, so the shortest witness is a single step"
    );
  }

  #[test]
  fn test_liveness_of_dead_has_no_example() {
    let mut net = PetriNet::default();
    let p1 = net.add_place();

    let mut t1_arcs = Arc::default();
    t1_arcs.add_input(p1, ArcKind::Consume(Weight(1)));
    let t1 = net.add_transition(t1_arcs);

    let m0 = net.initial_marking(vec![0]);
    let graph = net.reachable_markings(&m0);

    assert!(
      liveness_of(&graph, &m0, t1).example.is_empty(),
      "a dead transition has no firing sequence to show"
    );
  }

  #[test]
  fn test_liveness_of_total_on_two_node_cycle() {
    // p1 -t1-> p2 -t2-> p1: from either marking you can eventually fire either
    // transition, so both are L4 (Total), the strongest level.
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

    assert_eq!(liveness_of(&graph, &m0, t1).level, Liveness::Total);
    assert_eq!(liveness_of(&graph, &m0, t2).level, Liveness::Total);
  }

  #[test]
  fn test_liveness_of_repeatable_forever_but_not_total() {
    // p1 branches: t_dead runs into a dead end (p4), t_start feeds a p2<->p3
    // cycle (t2/t3) that repeats forever. t2 is on the cycle reachable from
    // the initial marking, but the dead-end branch can never fire it again,
    // so t2 is RepeatableForever without being Total.
    let mut net = PetriNet::default();
    let p1 = net.add_place();
    let p2 = net.add_place();
    let p3 = net.add_place();
    let p4 = net.add_place();

    let mut t_dead_arcs = Arc::default();
    t_dead_arcs
      .add_input(p1, ArcKind::Consume(Weight(1)))
      .add_output(p4, Weight(1));
    net.add_transition(t_dead_arcs);

    let mut t_start_arcs = Arc::default();
    t_start_arcs
      .add_input(p1, ArcKind::Consume(Weight(1)))
      .add_output(p2, Weight(1));
    let t_start = net.add_transition(t_start_arcs);

    let mut t2_arcs = Arc::default();
    t2_arcs
      .add_input(p2, ArcKind::Consume(Weight(1)))
      .add_output(p3, Weight(1));
    let t2 = net.add_transition(t2_arcs);

    let mut t3_arcs = Arc::default();
    t3_arcs
      .add_input(p3, ArcKind::Consume(Weight(1)))
      .add_output(p2, Weight(1));
    net.add_transition(t3_arcs);

    let m0 = net.initial_marking(vec![1, 0, 0, 0]);
    let graph = net.reachable_markings(&m0);

    let report = liveness_of(&graph, &m0, t2);

    assert_eq!(report.level, Liveness::RepeatableForever);
    assert_eq!(
      report.example,
      vec![t_start, t2],
      "shortest path from m0 that fires t2: t_start then t2"
    );
  }

  #[test]
  fn test_liveness_report_dead() {
    let mut net = PetriNet::default();
    let p1 = net.add_place();

    let mut t1_arcs = Arc::default();
    t1_arcs.add_input(p1, ArcKind::Consume(Weight(1)));
    let t1 = net.add_transition(t1_arcs);

    let m0 = net.initial_marking(vec![0]);
    let report = liveness_report(&net, &m0);

    assert_eq!(report.len(), 1);
    assert_eq!(report[&t1].level, Liveness::Dead);
    assert_eq!(report[&t1].k, Some(0));
  }

  #[test]
  fn test_liveness_report_matches_liveness_of_per_transition() {
    // Same branching net as test_liveness_of_repeatable_forever_but_not_total:
    // a dead-end branch (t_dead) and a branch that feeds a p2<->p3 cycle
    // (t_start, t2, t3). Nothing here is Total, since the dead-end branch
    // can't reach t_start/t2/t3 and the cycle branch can't reach t_dead.
    let mut net = PetriNet::default();
    let p1 = net.add_place();
    let p2 = net.add_place();
    let p3 = net.add_place();
    let p4 = net.add_place();

    let mut t_dead_arcs = Arc::default();
    t_dead_arcs
      .add_input(p1, ArcKind::Consume(Weight(1)))
      .add_output(p4, Weight(1));
    let t_dead = net.add_transition(t_dead_arcs);

    let mut t_start_arcs = Arc::default();
    t_start_arcs
      .add_input(p1, ArcKind::Consume(Weight(1)))
      .add_output(p2, Weight(1));
    let t_start = net.add_transition(t_start_arcs);

    let mut t2_arcs = Arc::default();
    t2_arcs
      .add_input(p2, ArcKind::Consume(Weight(1)))
      .add_output(p3, Weight(1));
    let t2 = net.add_transition(t2_arcs);

    let mut t3_arcs = Arc::default();
    t3_arcs
      .add_input(p3, ArcKind::Consume(Weight(1)))
      .add_output(p2, Weight(1));
    let t3 = net.add_transition(t3_arcs);

    let m0 = net.initial_marking(vec![1, 0, 0, 0]);
    let graph = net.reachable_markings(&m0);
    let report = liveness_report(&net, &m0);

    assert_eq!(report.len(), 4);
    for t in [t_dead, t_start, t2, t3] {
      assert_eq!(
        report[&t].level,
        liveness_of(&graph, &m0, t).level,
        "liveness_report disagrees with liveness_of for {t:?}"
      );
    }
    assert_eq!(report[&t_dead].level, Liveness::PotentiallyFirable);
    assert_eq!(report[&t_start].level, Liveness::PotentiallyFirable);
    assert_eq!(report[&t2].level, Liveness::RepeatableForever);
    assert_eq!(report[&t3].level, Liveness::RepeatableForever);
  }
}
