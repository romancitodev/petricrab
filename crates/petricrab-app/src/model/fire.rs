use super::{ArcKind, Marking, PetriNet, TransitionId};

#[derive(Debug, PartialEq, Eq)]
pub enum FireError {
    NotEnabled,
}

fn is_enabled(net: &PetriNet, marking: &Marking, t: TransitionId) -> bool {
    net.inputs(t).iter().all(|(place, kind)| {
        let tokens = marking.get(place).copied().unwrap_or(0);
        match kind {
            ArcKind::Consume(w) | ArcKind::Peek(w) => tokens >= *w,
            ArcKind::Inhibit(w) => tokens < *w,
        }
    })
}

pub fn enabled_transitions(net: &PetriNet, marking: &Marking) -> Vec<TransitionId> {
    net.transition_ids()
        .filter(|&t| is_enabled(net, marking, t))
        .collect()
}

/// Fires `t` against `marking`, returning the resulting marking without mutating `net`.
/// Only `ArcKind::Consume` input arcs remove tokens; `Peek`/`Inhibit` only gate enabling.
pub fn fire_in(net: &PetriNet, marking: &Marking, t: TransitionId) -> Result<Marking, FireError> {
    if !is_enabled(net, marking, t) {
        return Err(FireError::NotEnabled);
    }
    let mut next = marking.clone();
    for (place, kind) in net.inputs(t) {
        if let ArcKind::Consume(w) = kind {
            *next.entry(*place).or_insert(0) -= w;
        }
    }
    for (place, weight) in net.outputs(t) {
        *next.entry(*place).or_insert(0) += weight;
    }
    Ok(next)
}

/// Fires `t` against the net's current marking, mutating it in place.
pub fn fire(net: &mut PetriNet, t: TransitionId) -> Result<(), FireError> {
    let next = fire_in(net, &net.marking(), t)?;
    net.set_marking(&next);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_disabled_until_enough_tokens() {
        let mut net = PetriNet::new();
        let p = net.add_place("p");
        let t = net.add_transition("t");
        net.add_arc_place_to_transition(p, t, ArcKind::Consume(2))
            .unwrap();

        assert!(enabled_transitions(&net, &net.marking()).is_empty());

        net.set_tokens(p, 2);
        assert_eq!(enabled_transitions(&net, &net.marking()), vec![t]);
    }

    #[test]
    fn fire_in_does_not_mutate_original_marking() {
        let mut net = PetriNet::new();
        let p_in = net.add_place("in");
        let p_out = net.add_place("out");
        let t = net.add_transition("t");
        net.add_arc_place_to_transition(p_in, t, ArcKind::Consume(1))
            .unwrap();
        net.add_arc_transition_to_place(t, p_out, 1).unwrap();
        net.set_tokens(p_in, 1);

        let original = net.marking();
        let next = fire_in(&net, &original, t).unwrap();

        assert_eq!(original.get(&p_in), Some(&1));
        assert_eq!(next.get(&p_in), Some(&0));
        assert_eq!(next.get(&p_out), Some(&1));
    }

    #[test]
    fn fire_mutates_net_marking() {
        let mut net = PetriNet::new();
        let p_in = net.add_place("in");
        let p_out = net.add_place("out");
        let t = net.add_transition("t");
        net.add_arc_place_to_transition(p_in, t, ArcKind::Consume(1))
            .unwrap();
        net.add_arc_transition_to_place(t, p_out, 1).unwrap();
        net.set_tokens(p_in, 1);

        fire(&mut net, t).unwrap();

        assert_eq!(net.tokens(p_in), 0);
        assert_eq!(net.tokens(p_out), 1);
    }

    #[test]
    fn fire_disabled_transition_errors() {
        let mut net = PetriNet::new();
        let p = net.add_place("p");
        let t = net.add_transition("t");
        net.add_arc_place_to_transition(p, t, ArcKind::Consume(1))
            .unwrap();

        assert_eq!(fire(&mut net, t), Err(FireError::NotEnabled));
    }

    #[test]
    fn peek_arc_requires_tokens_but_does_not_consume() {
        let mut net = PetriNet::new();
        let guard = net.add_place("guard");
        let t = net.add_transition("t");
        net.add_arc_place_to_transition(guard, t, ArcKind::Peek(1))
            .unwrap();
        net.set_tokens(guard, 1);

        fire(&mut net, t).unwrap();

        assert_eq!(net.tokens(guard), 1);
    }

    #[test]
    fn inhibit_arc_blocks_when_tokens_present() {
        let mut net = PetriNet::new();
        let guard = net.add_place("guard");
        let t = net.add_transition("t");
        net.add_arc_place_to_transition(guard, t, ArcKind::Inhibit(1))
            .unwrap();

        assert_eq!(enabled_transitions(&net, &net.marking()), vec![t]);

        net.set_tokens(guard, 1);
        assert!(enabled_transitions(&net, &net.marking()).is_empty());
    }
}
