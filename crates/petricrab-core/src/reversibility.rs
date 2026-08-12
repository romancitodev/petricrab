use std::collections::{BTreeMap, BTreeSet};

use crate::liveness::can_reach;
use crate::marking::Marking;
use crate::net::{PetriNet, TransitionId};

/// `candidate` is a home state of `graph` if every marking in it can reach `candidate`
/// i.e. no matter how far you wander from the initial marking, you can always get back to
/// `candidate`. Generic over the marking type, same as `liveness`'s traversal helpers, so it
/// runs over either the exact reachability graph or the coverability graph.
fn is_home_state<M: Ord + Clone>(
  graph: &BTreeMap<M, Vec<(TransitionId, M)>>,
  candidate: &M,
) -> bool {
  graph.keys().all(|m| can_reach(graph, m, candidate))
}

/// A net is reversible if you can always get back to the initial marking which is exactly
/// saying `initial_marking` itself is a home state (see [`home_states`]).
///
/// # Panics
///
/// Same caveat as [`crate::net::PetriNet::reachable_markings`]: this never returns if the net
/// is unbounded, since `R(M0)` itself is infinite. Use [`is_reversible_covering`] instead when
/// that's a possibility.
pub fn is_reversible(net: &PetriNet, initial_marking: &Marking) -> bool {
  let graph = net.reachable_markings(initial_marking);
  is_home_state(&graph, initial_marking)
}

/// Whether `initial_marking` is a home state, computed over the (always-finite) Karp-Miller
/// coverability graph instead of the exact reachability set (see
/// `liveness::liveness_report_covering` for the same technique and its caveats). This one stays
/// exact, not just "good enough": the coverability graph's root is always the literal, concrete
/// `initial_marking` (only its descendants can widen to Ω), so the question stays precise
/// regardless of what's unbounded elsewhere in the net.
pub fn is_reversible_covering(net: &PetriNet, initial_marking: &Marking) -> bool {
  let graph = crate::coverability::coverability_graph(net, initial_marking);
  let initial: crate::coverability::ExtendedMarking = initial_marking.into();
  is_home_state(&graph, &initial)
}

/// Every home state reachable from `initial_marking`: markings that every other marking in
/// `R(M0)` can always get back to. A net doesn't need to be reversible to have one. See
/// [`is_reversible`] for the stricter "always get back to `M0` specifically" property.
///
/// # Panics
///
/// Same caveat as [`crate::net::PetriNet::reachable_markings`]: this never returns if the net
/// is unbounded, since `R(M0)` itself is infinite.
pub fn home_states(net: &PetriNet, initial_marking: &Marking) -> BTreeSet<Marking> {
  let graph = net.reachable_markings(initial_marking);
  graph
    .keys()
    .filter(|candidate| is_home_state(&graph, candidate))
    .cloned()
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::{Arc, ArcKind, Weight};

  #[test]
  fn test_two_node_cycle_is_reversible_and_every_marking_is_a_home_state() {
    // p1<->p2, 1 token bouncing forever: from either marking you can always get back to
    // the other (and to M0), so it's reversible and both markings qualify as home states.
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

    assert!(is_reversible(&net, &initial));
    assert_eq!(
      home_states(&net, &initial).len(),
      2,
      "both markings in a 2-cycle are home states"
    );
  }

  #[test]
  fn test_h2o_is_not_reversible_but_has_a_home_state() {
    // Same reaction as net.rs's make_h2o: 2H + 1O -> H2O, no way back. R(M0) is just
    // {[2,2,0], [0,1,1]}, and the dead end [0,1,1] can't reach M0. not reversible.
    // But [0,1,1] IS a home state: every marking (including itself) can reach it, since the
    // net has nowhere else to go. This is exactly the "relax reversibility to a home state"
    // example from the text: you can't always get back to M0, but you can always get back
    // to *some* fixed state.
    let mut net = PetriNet::default();
    let h = net.add_place();
    let o = net.add_place();
    let h2o = net.add_place();

    let mut arcs = Arc::default();
    arcs
      .add_input(h, ArcKind::Consume(Weight(2)))
      .add_input(o, ArcKind::Consume(Weight(1)))
      .add_output(h2o, Weight(1));
    net.add_transition(arcs);

    let initial = net.initial_marking(vec![2, 2, 0]);
    let dead_end = Marking::new(vec![0, 1, 1]);

    assert!(!is_reversible(&net, &initial));

    let homes = home_states(&net, &initial);
    assert!(homes.contains(&dead_end), "the dead end is a home state");
    assert!(
      !homes.contains(&initial),
      "M0 is not a home state here — that's exactly why it's not reversible"
    );
  }
}
