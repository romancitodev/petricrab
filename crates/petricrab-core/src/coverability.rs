use std::{
  collections::BTreeMap,
  ops::{Add, Sub},
};

use crate::{Arc, ArcKind, Marking, PetriNet, TransitionId, Weight, net::PlaceId};

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy)]
pub(crate) enum ExtendedToken {
  Finite(usize),
  Omega,
}

#[derive(Default, PartialEq, Eq, PartialOrd, Ord, Debug, Clone)]
pub(crate) struct ExtendedMarking(Vec<ExtendedToken>);

impl From<Marking> for ExtendedMarking {
  fn from(marking: Marking) -> Self {
    Self(
      marking
        .0
        .into_iter()
        .map(|t| ExtendedToken::Finite(t))
        .collect(),
    )
  }
}

impl From<&Marking> for ExtendedMarking {
  fn from(marking: &Marking) -> Self {
    Self(
      marking
        .0
        .iter()
        .map(|t| ExtendedToken::Finite(*t))
        .collect(),
    )
  }
}

impl ExtendedMarking {
  fn new(extended_tokens: Vec<ExtendedToken>) -> Self {
    Self(extended_tokens)
  }

  pub fn tokens(&self, id: PlaceId) -> ExtendedToken {
    self.0[id.0]
  }

  pub fn tokens_mut(&mut self, id: PlaceId) -> &mut ExtendedToken {
    &mut self.0[id.0]
  }

  /// Karp-Miller definition:
  ///
  /// check if `self` dominates `other` by this rules:
  /// - `self` and `other` have the same number of tokens
  /// - for each token, `self` is greater than or equal to `other`
  /// - at least one token is strictly greater than `other`
  fn dominates(&self, other: &Self) -> bool {
    let mut any_greater = false;
    for (a, b) in self.0.iter().zip(&other.0) {
      if a < b {
        return false;
      }
      any_greater |= a > b;
    }
    any_greater
  }

  /// Given that `self` dominates `other` (see [`Self::dominates`]), return the marking with
  /// `Omega` in every component that strictly grew from `other` to `self` — the Karp-Miller
  /// widening step. Components that stayed equal keep their concrete value.
  fn promote(&self, other: &Self) -> Self {
    Self(
      self
        .0
        .iter()
        .zip(&other.0)
        .map(|(a, b)| if a > b { ExtendedToken::Omega } else { *a })
        .collect(),
    )
  }
}

/// A transition is said to be enabled if:
///
/// Each input place of the transition is marked with at least w(p,t) tokens, where w(p,t)
/// is the weight of the arc from p to t.
///
/// In other words, a transition is enabled if all its input places have enough tokens to satisfy the weights of the arcs leading to it.
fn is_enabled(marking: &ExtendedMarking, arcs: &Arc) -> bool {
  arcs.inputs().iter().all(|(id, kind)| match kind {
    ArcKind::Consume(Weight(w)) => marking.tokens(*id) >= ExtendedToken::Finite(*w),
    ArcKind::Peek => marking.tokens(*id) > ExtendedToken::Finite(0),
    ArcKind::Inhibit => marking.tokens(*id) == ExtendedToken::Finite(0),
  })
}

/// Fire `arcs` against `marking`: consume from inputs, then produce into outputs. Mirrors
/// [`crate::net::Transition::fire`], but over `ExtendedToken`, so a place already at `Omega`
/// stays `Omega` through the arithmetic instead of overflowing.
///
/// Returns `false` (and leaves the marking untouched) if `arcs` is not enabled.
fn fire(marking: &mut ExtendedMarking, arcs: &Arc) -> bool {
  if !is_enabled(marking, arcs) {
    return false;
  }

  consume_tokens(marking, arcs);
  forward_tokens(marking, arcs);

  true
}

fn consume_tokens(marking: &mut ExtendedMarking, arcs: &Arc) {
  for (id, kind) in arcs.inputs() {
    let ArcKind::Consume(Weight(w)) = kind else {
      continue;
    };
    let current = marking.tokens(*id);
    *marking.tokens_mut(*id) = current - ExtendedToken::Finite(*w);
  }
}

fn forward_tokens(marking: &mut ExtendedMarking, arcs: &Arc) {
  for (id, weight) in arcs.output() {
    let current = marking.tokens(*id);
    *marking.tokens_mut(*id) = current + ExtendedToken::Finite(weight.0);
  }
}

impl Add for ExtendedToken {
  type Output = Self;

  fn add(self, rhs: Self) -> Self::Output {
    match (self, rhs) {
      (ExtendedToken::Finite(a), ExtendedToken::Finite(b)) => ExtendedToken::Finite(a + b),
      (ExtendedToken::Finite(_), ExtendedToken::Omega) => ExtendedToken::Omega,
      (ExtendedToken::Omega, _) => ExtendedToken::Omega,
    }
  }
}

impl Sub for ExtendedToken {
  type Output = Self;

  fn sub(self, rhs: Self) -> Self::Output {
    match (self, rhs) {
      (ExtendedToken::Finite(a), ExtendedToken::Finite(b)) => {
        ExtendedToken::Finite(a.saturating_sub(b))
      }
      (ExtendedToken::Finite(_), ExtendedToken::Omega) => ExtendedToken::Omega,
      (ExtendedToken::Omega, _) => ExtendedToken::Omega,
    }
  }
}

/// Karp-Miller coverability graph rooted at `initial_marking`.
///
/// Shaped just like [`crate::net::PetriNet::reachable_markings`] (same
/// `BTreeMap<marking, edges>` return type, same iterative-DFS-with-a-stack skeleton), but over
/// `ExtendedMarking` instead of `Marking`: whenever a newly fired marking dominates an ancestor
/// on its own path from the root, the components that strictly grew are widened to `Omega`.
/// That widening is what guarantees this terminates even for nets where
/// `reachable_markings` would hang forever.
pub(crate) fn coverability_graph(
  net: &PetriNet,
  initial: &Marking,
) -> BTreeMap<ExtendedMarking, Vec<(TransitionId, ExtendedMarking)>> {
  let mut visited = BTreeMap::new();

  // Stack frame = (marking to expand, its ancestors from the root — NOT including itself).
  // NOTE: your old type here was `Vec<(TransitionId, ExtendedMarking)>`, which is the shape
  // of an *edge list* (what `visited`'s values hold), not a *path of ancestor markings*. The
  // widening step below needs the markings themselves, not the transitions that produced them.
  let mut stack: Vec<(ExtendedMarking, Vec<ExtendedMarking>)> = vec![(initial.into(), Vec::new())];

  while let Some((current, path)) = stack.pop() {
    if visited.contains_key(&current) {
      // Rule 1 (exact duplicate, GLOBAL dedup): we've already expanded this exact marking
      // somewhere in the graph — firing only depends on the current marking, not on how we
      // got here, so its future is identical regardless of branch. Same trick as
      // `reachable_markings`.
      continue;
    }
    visited.insert(current.clone(), Vec::new());

    // `current` is itself a valid ancestor for its OWN children (a child can dominate its
    // direct parent — the shortest possible growth cycle), so it joins the path here, before
    // we start generating successors. `chain(once(&current))` builds that view without
    // allocating a new Vec or cloning `current` — it's just references over `path` + one more.
    let ancestors = || path.iter().chain(std::iter::once(&current));

    for t_id in net.transition_ids() {
      let arcs = net.transition(t_id).arcs();
      if !is_enabled(&current, arcs) {
        continue;
      }

      let mut new_marking = current.clone();
      fire(&mut new_marking, arcs);

      // Rule 2 (widening, ONLY against `ancestors` — never against the global `visited`
      // set): for every ancestor that `new_marking` dominates, promote the strictly-grown
      // components to Omega. Checking every ancestor (not stopping at the first match) is
      // safe because `promote` only ever adds more Omega, never removes one, so the order
      // doesn't change the final result.
      for ancestor in ancestors() {
        if new_marking.dominates(ancestor) {
          new_marking = new_marking.promote(ancestor);
        }
      }

      visited
        .get_mut(&current)
        .unwrap()
        .push((t_id, new_marking.clone()));

      // The child needs to OWN its path (it'll outlive this stack frame), so this is the one
      // place a real clone of the ancestor data is unavoidable — everything above was just
      // borrowing.
      let next_path = ancestors().cloned().collect();
      stack.push((new_marking, next_path));
    }
  }

  visited
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn check_ord() {
    assert!(ExtendedToken::Finite(1) < ExtendedToken::Finite(2));
    assert!(ExtendedToken::Finite(1) < ExtendedToken::Omega);
    assert!(ExtendedToken::Finite(999) < ExtendedToken::Omega);
    assert!(ExtendedToken::Finite(2) > ExtendedToken::Finite(1));
    assert!(ExtendedToken::Omega > ExtendedToken::Finite(1));
    assert!(ExtendedToken::Omega > ExtendedToken::Finite(1_000_000));
  }

  #[test]
  fn check_ops() {
    assert_eq!(
      ExtendedToken::Finite(1) + ExtendedToken::Finite(2),
      ExtendedToken::Finite(3)
    );
    assert_eq!(
      ExtendedToken::Finite(1) + ExtendedToken::Omega,
      ExtendedToken::Omega
    );
    assert_eq!(
      ExtendedToken::Omega + ExtendedToken::Finite(1),
      ExtendedToken::Omega
    );
    assert_eq!(
      ExtendedToken::Finite(1) - ExtendedToken::Finite(2),
      ExtendedToken::Finite(0)
    );
    assert_eq!(
      ExtendedToken::Finite(1) - ExtendedToken::Omega,
      ExtendedToken::Omega
    );
    assert_eq!(
      ExtendedToken::Omega - ExtendedToken::Finite(1),
      ExtendedToken::Omega
    );
  }

  #[test]
  fn test_dominates() {
    let a = ExtendedMarking::new(vec![ExtendedToken::Finite(1), ExtendedToken::Omega]);
    let b = ExtendedMarking::new(vec![ExtendedToken::Finite(1), ExtendedToken::Finite(2)]);
    assert!(a.dominates(&b)); // [1 == 1, omega > N]

    let c = ExtendedMarking::new(vec![ExtendedToken::Finite(2), ExtendedToken::Omega]);
    let d = ExtendedMarking::new(vec![ExtendedToken::Finite(1), ExtendedToken::Finite(2)]);
    assert!(!d.dominates(&c)); // [2 > 1, omega > N].

    let e = ExtendedMarking::new(vec![
      ExtendedToken::Omega,
      ExtendedToken::Omega,
      ExtendedToken::Finite(3),
    ]);
    let f = ExtendedMarking::new(vec![
      ExtendedToken::Omega,
      ExtendedToken::Omega,
      ExtendedToken::Finite(1),
    ]);
    assert!(e.dominates(&f)); // [omega == omega, omega == omega, 3 > 1]
  }

  #[test]
  fn test_fire_keeps_omega_and_produces_output() {
    // Same H2O shape as make_h2o test, but H is already Omega: firing should
    // consume O normally, leave H at Omega (Omega - Finite(w) = Omega), and produce H2O.
    let h = PlaceId(0);
    let o = PlaceId(1);
    let h2o = PlaceId(2);

    let mut arcs = Arc::default();
    arcs
      .add_input(h, ArcKind::Consume(Weight(2)))
      .add_input(o, ArcKind::Consume(Weight(1)))
      .add_output(h2o, Weight(1));

    let mut marking = ExtendedMarking::new(vec![
      ExtendedToken::Omega,
      ExtendedToken::Finite(2),
      ExtendedToken::Finite(0),
    ]);

    assert!(is_enabled(&marking, &arcs));
    assert!(fire(&mut marking, &arcs));

    assert_eq!(
      marking.tokens(h),
      ExtendedToken::Omega,
      "omega absorbs consumption"
    );
    assert_eq!(marking.tokens(o), ExtendedToken::Finite(1));
    assert_eq!(
      marking.tokens(h2o),
      ExtendedToken::Finite(1),
      "output must be produced"
    );
  }

  #[test]
  fn test_fire_not_enabled_leaves_marking_untouched() {
    let p1 = PlaceId(0);
    let p2 = PlaceId(1);

    let mut arcs = Arc::default();
    arcs
      .add_input(p1, ArcKind::Consume(Weight(2)))
      .add_output(p2, Weight(1));

    let mut marking =
      ExtendedMarking::new(vec![ExtendedToken::Finite(1), ExtendedToken::Finite(0)]);

    assert!(!fire(&mut marking, &arcs));
    assert_eq!(marking.tokens(p1), ExtendedToken::Finite(1));
    assert_eq!(marking.tokens(p2), ExtendedToken::Finite(0));
  }

  #[test]
  fn test_promote_only_strictly_growing_components_become_omega() {
    let ancestor = ExtendedMarking::new(vec![ExtendedToken::Finite(1), ExtendedToken::Finite(3)]);
    let descendant = ExtendedMarking::new(vec![ExtendedToken::Finite(2), ExtendedToken::Finite(3)]);
    assert!(descendant.dominates(&ancestor));

    let promoted = descendant.promote(&ancestor);

    assert_eq!(
      promoted.tokens(PlaceId(0)),
      ExtendedToken::Omega,
      "strictly grew from 1 to 2 -> omega"
    );
    assert_eq!(
      promoted.tokens(PlaceId(1)),
      ExtendedToken::Finite(3),
      "unchanged component stays concrete, not just 'the max'"
    );
  }

  #[test]
  fn test_coverability_graph_terminates_on_unbounded_source() {
    // t: () -> p1, a source transition (see Transition::is_source) with no input. Firing it
    // forever would pump p1 to infinity — exactly the net `reachable_markings` would hang on.
    // If this test finishes at all, that's half the proof; the other half is the assertion.
    let mut net = PetriNet::default();
    let p1 = net.add_place();

    let mut arcs = Arc::default();
    arcs.add_output(p1, Weight(1));
    net.add_transition(arcs);

    let initial = net.initial_marking(vec![0]);
    let graph = coverability_graph(&net, &initial);

    assert!(
      graph.keys().any(|m| m.tokens(p1) == ExtendedToken::Omega),
      "p1 must be widened to Omega instead of growing without bound"
    );
    assert_eq!(
      graph.len(),
      2,
      "root [0] plus its widened child [Omega] — nothing else should ever be generated"
    );
  }

  #[test]
  fn test_coverability_graph_bounded_cycle_has_no_omega() {
    // Same p1<->p2 cycle as net.rs's test_reachable_markings_cycle: bounded, so the
    // coverability graph must match the plain reachability graph exactly — no widening.
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

    let initial = net.initial_marking(vec![1, 0]);
    let graph = coverability_graph(&net, &initial);

    assert_eq!(graph.len(), 2, "bounded cycle: only 2 reachable markings");
    assert!(
      graph
        .keys()
        .all(|m| !m.0.iter().any(|t| *t == ExtendedToken::Omega)),
      "a bounded net must never produce Omega"
    );
  }
}
