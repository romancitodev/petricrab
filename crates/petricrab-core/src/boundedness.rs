use std::collections::BTreeMap;

use crate::{
  Marking, PetriNet,
  coverability::{ExtendedToken, coverability_graph},
  net::PlaceId,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Boundedness {
  /// Max *k* tokens seen.
  Bounded(usize),
  Unbounded,
}

impl Boundedness {
  /// A place is safe if it's 1-bounded — 0 or 1 tokens in every reachable (or covered) marking.
  pub fn is_safe(&self) -> bool {
    matches!(self, Boundedness::Bounded(k) if *k <= 1)
  }
}

/// Boundedness of every place in `net`, over the Karp-Miller coverability graph rooted at
/// `initial_marking`. A place is [`Boundedness::Unbounded`] if `Omega` ever appears there in
/// the graph; otherwise it's [`Boundedness::Bounded`] with the max token count seen at that
/// place across every reachable (or covered) marking.
///
/// A whole net is bounded/safe iff every entry in the returned map is.
pub fn boundedness_report(
  net: &PetriNet,
  initial_marking: &Marking,
) -> BTreeMap<PlaceId, Boundedness> {
  let graph = coverability_graph(net, initial_marking);

  net
    .place_ids()
    .map(|place| {
      let boundedness = graph.keys().fold(Boundedness::Bounded(0), |acc, marking| {
        match (acc, marking.tokens(place)) {
          (Boundedness::Unbounded, _) | (_, ExtendedToken::Omega) => Boundedness::Unbounded,
          (Boundedness::Bounded(k), ExtendedToken::Finite(n)) => Boundedness::Bounded(k.max(n)),
        }
      });
      (place, boundedness)
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::{Arc, ArcKind, Weight};

  #[test]
  fn test_boundedness_report_h2o() {
    // Same reaction as net.rs's make_h2o: consumes 2H + 1O -> 1 H2O. Starting at 2H/2O/0,
    // nothing ever grows past its initial count, so every place is bounded by its own start.
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
    let report = boundedness_report(&net, &initial);

    assert_eq!(report[&h], Boundedness::Bounded(2));
    assert_eq!(report[&o], Boundedness::Bounded(2));
    assert_eq!(report[&h2o], Boundedness::Bounded(1));
    assert!(!report[&h].is_safe(), "2 tokens is bounded but not safe");
  }

  #[test]
  fn test_boundedness_report_cycle_is_safe() {
    // p1<->p2, 1 token bouncing between them forever: both places are 1-bounded, so the
    // whole net is safe.
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
    let report = boundedness_report(&net, &initial);

    assert_eq!(report[&p1], Boundedness::Bounded(1));
    assert_eq!(report[&p2], Boundedness::Bounded(1));
    assert!(
      report.values().all(Boundedness::is_safe),
      "whole net is safe"
    );
  }

  #[test]
  fn test_boundedness_report_source_transition_is_unbounded() {
    // t: () -> p1, no input: p1 grows forever, so it must report Unbounded, not some huge k.
    let mut net = PetriNet::default();
    let p1 = net.add_place();

    let mut arcs = Arc::default();
    arcs.add_output(p1, Weight(1));
    net.add_transition(arcs);

    let initial = net.initial_marking(vec![0]);
    let report = boundedness_report(&net, &initial);

    assert_eq!(report[&p1], Boundedness::Unbounded);
    assert!(!report[&p1].is_safe());
  }
}
