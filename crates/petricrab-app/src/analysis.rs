use std::collections::{HashMap, VecDeque};

use crate::model::fire::{enabled_transitions, fire_in};
use crate::model::{
    ArcKind as ModelArcKind, Marking as ModelMarking, PetriNet as ModelNet,
    PlaceId as ModelPlaceId, TransitionId as ModelTransitionId,
};

/// Safety valve only — no longer the signal for "this net is unbounded" (see `explore`).
const MAX_STATES: usize = 200_000;

pub struct StateEdge {
    pub from: usize,
    pub via: ModelTransitionId,
    pub to: usize,
}

pub struct StateGraph {
    pub nodes: Vec<ModelMarking>,
    pub edges: Vec<StateEdge>,
}

pub enum ExploreError {
    /// Karp-Miller proved these places grow without bound — no point enumerating states.
    Unbounded(Vec<ModelPlaceId>),
    /// Bounded, but the exact reachable set is bigger than the safety cap.
    TooManyStates,
}

/// Builds a fresh `petricrab_core::PetriNet` + initial marking from the current editable net,
/// mapping every place/transition to a slot by iteration order. One-way and read-only: the
/// editable model (labels, slotmap ids, removal) never needs to round-trip back through this —
/// only the analysis results (keyed by `petricrab_core::PlaceId`/`TransitionId`) do, via the
/// returned map.
///
/// `ponytail:` `petricrab_core::ArcKind::{Peek, Inhibit}` follow Murata's classic *unweighted*
/// definition ("some token present" / "no token present"), not the editable model's more general
/// `Peek(w)`/`Inhibit(w)`. Every weight above 1 degrades to the classic case here. This is safe
/// today because `editor.rs`'s weight stepper caps at 1 for those two arc kinds specifically —
/// upgrade path: give `petricrab_core::ArcKind` a weighted Peek/Inhibit if the app ever needs to
/// let users raise that cap.
struct AnalysisNet {
    net: petricrab_core::PetriNet,
    initial: petricrab_core::Marking,
    place_map: HashMap<ModelPlaceId, petricrab_core::PlaceId>,
    transition_map: HashMap<ModelTransitionId, petricrab_core::TransitionId>,
}

fn to_analysis_net(net: &ModelNet, marking: &ModelMarking) -> AnalysisNet {
    let mut analysis_net = petricrab_core::PetriNet::default();
    let mut place_map = HashMap::new();
    let mut transition_map = HashMap::new();
    let mut tokens = Vec::new();

    for p in net.place_ids() {
        place_map.insert(p, analysis_net.add_place());
        tokens.push(*marking.get(&p).unwrap_or(&0) as usize);
    }

    for t in net.transition_ids() {
        let mut arcs = petricrab_core::Arc::default();
        for &(p, kind) in net.inputs(t) {
            let Some(&place) = place_map.get(&p) else {
                continue;
            };
            let mapped = match kind {
                ModelArcKind::Consume(w) => {
                    petricrab_core::ArcKind::Consume(petricrab_core::Weight::new(w as usize))
                }
                ModelArcKind::Peek(_) => petricrab_core::ArcKind::Peek,
                ModelArcKind::Inhibit(_) => petricrab_core::ArcKind::Inhibit,
            };
            arcs.add_input_inplace(place, mapped);
        }
        for &(p, weight) in net.outputs(t) {
            let Some(&place) = place_map.get(&p) else {
                continue;
            };
            arcs.add_output_inplace(place, petricrab_core::Weight::new(weight as usize));
        }
        let analysis_t = analysis_net.add_transition(arcs);
        transition_map.insert(t, analysis_t);
    }

    let initial = analysis_net.initial_marking(tokens);
    AnalysisNet {
        net: analysis_net,
        initial,
        place_map,
        transition_map,
    }
}

/// Places that Karp-Miller's coverability graph proved unbounded, translated back to the
/// editable model's own place ids so the GUI can point at them directly.
pub fn unbounded_places(net: &ModelNet, marking: &ModelMarking) -> Vec<ModelPlaceId> {
    let analysis = to_analysis_net(net, marking);
    let report = petricrab_core::boundedness_report(&analysis.net, &analysis.initial);

    analysis
        .place_map
        .into_iter()
        .filter(|(_, analysis_id)| {
            matches!(
                report.get(analysis_id),
                Some(petricrab_core::Boundedness::Unbounded)
            )
        })
        .map(|(model_id, _)| model_id)
        .collect()
}

/// Boundedness/liveness/reversibility for the whole net, over its finite `R(M0)`. `Err` if the
/// net is unbounded — none of these three properties can be computed on an infinite state space
/// with the primitives this app has today (boundedness is the one exception, which is exactly
/// how it's able to tell you *that* it's unbounded in the first place).
pub struct NetProperties {
    pub boundedness: Vec<(ModelPlaceId, petricrab_core::Boundedness)>,
    pub safe: bool,
    pub liveness: Vec<(ModelTransitionId, petricrab_core::Liveness)>,
    pub reversible: bool,
    pub home_state_count: usize,
}

pub fn analyze(net: &ModelNet, marking: &ModelMarking) -> Result<NetProperties, Vec<ModelPlaceId>> {
    let unbounded = unbounded_places(net, marking);
    if !unbounded.is_empty() {
        return Err(unbounded);
    }

    let analysis = to_analysis_net(net, marking);
    let boundedness_report = petricrab_core::boundedness_report(&analysis.net, &analysis.initial);
    let liveness_report = petricrab_core::liveness_report(&analysis.net, &analysis.initial);

    let boundedness: Vec<_> = analysis
        .place_map
        .iter()
        .map(|(&model_p, analysis_p)| (model_p, boundedness_report[analysis_p]))
        .collect();
    let safe = boundedness.iter().all(|(_, b)| b.is_safe());

    let liveness: Vec<_> = analysis
        .transition_map
        .iter()
        .map(|(&model_t, analysis_t)| (model_t, liveness_report[analysis_t].level))
        .collect();

    let reversible = petricrab_core::is_reversible(&analysis.net, &analysis.initial);
    let home_state_count = petricrab_core::home_states(&analysis.net, &analysis.initial).len();

    Ok(NetProperties {
        boundedness,
        safe,
        liveness,
        reversible,
        home_state_count,
    })
}

/// Explores the reachability set from `initial` via breadth-first search — but checks
/// boundedness first (Karp-Miller, always terminates) instead of just running BFS until an
/// arbitrary state-count cap trips. An unbounded net gets a precise answer (which places grow
/// forever) instead of a "got this far, no idea why" warning.
pub fn explore(net: &ModelNet, initial: &ModelMarking) -> Result<StateGraph, ExploreError> {
    let unbounded = unbounded_places(net, initial);
    if !unbounded.is_empty() {
        return Err(ExploreError::Unbounded(unbounded));
    }

    let mut nodes = vec![initial.clone()];
    let mut edges = Vec::new();
    let mut index_of: HashMap<ModelMarking, usize> = HashMap::from([(initial.clone(), 0)]);
    let mut queue = VecDeque::from([0usize]);

    while let Some(from) = queue.pop_front() {
        let marking = nodes[from].clone();
        for t in enabled_transitions(net, &marking) {
            let next = fire_in(net, &marking, t).expect("transition was reported enabled");
            let to = match index_of.get(&next) {
                Some(&existing) => existing,
                None => {
                    if nodes.len() >= MAX_STATES {
                        return Err(ExploreError::TooManyStates);
                    }
                    let new_index = nodes.len();
                    nodes.push(next.clone());
                    index_of.insert(next, new_index);
                    queue.push_back(new_index);
                    new_index
                }
            };
            edges.push(StateEdge { from, via: t, to });
        }
    }

    Ok(StateGraph { nodes, edges })
}
