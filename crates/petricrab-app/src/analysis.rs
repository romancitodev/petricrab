use std::collections::{HashMap, VecDeque};

use crate::model::fire::{enabled_transitions, fire_in};
use crate::model::{
  ArcKind as ModelArcKind, Marking as ModelMarking, PetriNet as ModelNet, PlaceId as ModelPlaceId,
  TransitionId as ModelTransitionId,
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

/// Liveness of one transition, plus a witness: the shortest firing sequence from `M0` that
/// actually fires it (empty for `Dead`, since there is none).
pub struct TransitionLiveness {
  pub transition: ModelTransitionId,
  pub level: petricrab_core::Liveness,
  pub example: Vec<ModelTransitionId>,
}

pub struct NetBehavior {
  pub liveness: Vec<TransitionLiveness>,
  pub reversible: bool,
  /// The actual home state markings (see `petricrab_core::home_states`), not just a count —
  /// every one of these is a marking you can always get back to from anywhere in `R(M0)`. If
  /// `reversible` is true, `initial_marking` itself is always one of them.
  ///
  /// Only the full set when `precise` is true. Otherwise, if `reversible` is true, this is just
  /// `[initial_marking]`, not the full set.
  pub home_states: Vec<ModelMarking>,
  /// Dead markings reachable from `M0`. Same `precise` split as everything else here.
  pub deadlocks: Vec<Deadlock>,
  /// True if `liveness`/`reversible` came from the exact reachability set (the net is bounded).
  /// False means they came from `petricrab_core::{liveness_report_covering,
  /// is_reversible_covering}` instead, over the coverability graph. `reversible` stays exact
  /// either way, but `Liveness::RepeatableForever` can mean L2 (ArbitrarilyRepeatable) instead
  /// of true L3 in that case; see that function's doc.
  pub precise: bool,
}

/// The shortest firing sequence from `M0` to a dead marking (no enabled transitions), plus every
/// state along the way (`states[0] == M0`, `states[i + 1]` is `states[i]` after firing
/// `example[i]`). `states` can be shorter than `example.len() + 1` when it came from
/// `deadlocks_covering`: the coverability graph's edges aren't always witnessed by one real
/// firing sequence, so replaying `example` against the real net can dead-end early.
pub struct Deadlock {
  pub example: Vec<ModelTransitionId>,
  pub states: Vec<ModelMarking>,
}

/// Replays `path` from `from` against the real net, returning every state visited
/// (`states[0] == from`). Stops early (shorter than `path.len() + 1`) if some transition in
/// `path` turns out not to be enabled when actually fired — only possible for a witness that
/// came from a coverability graph instead of the exact reachability set.
pub fn replay_path(
  net: &ModelNet,
  from: &ModelMarking,
  path: &[ModelTransitionId],
) -> Vec<ModelMarking> {
  let mut states = vec![from.clone()];
  for &t in path {
    let Ok(next) = fire_in(net, states.last().unwrap(), t) else {
      break;
    };
    states.push(next);
  }
  states
}

/// Shortest firing sequence from `from` to `target`, found by exploring `R(from)` — `None` if
/// the net is unbounded/too big to explore, or `target` just isn't reachable. Powers "show the
/// route" for anything keyed by a marking instead of a ready-made witness (home states, a
/// clicked reachability-graph node), by reusing the same BFS `explore` already does.
pub fn path_to(
  net: &ModelNet,
  from: &ModelMarking,
  target: &ModelMarking,
) -> Option<(Vec<ModelMarking>, Vec<ModelTransitionId>)> {
  let state_graph = explore(net, from).ok()?;
  let target_idx = state_graph.nodes.iter().position(|m| m == target)?;

  let mut came_from: HashMap<usize, (ModelTransitionId, usize)> = HashMap::new();
  let mut visited = std::collections::HashSet::from([0usize]);
  let mut queue = VecDeque::from([0usize]);

  'bfs: while let Some(idx) = queue.pop_front() {
    for edge in state_graph.edges.iter().filter(|e| e.from == idx) {
      if visited.insert(edge.to) {
        came_from.insert(edge.to, (edge.via, idx));
        if edge.to == target_idx {
          break 'bfs;
        }
        queue.push_back(edge.to);
      }
    }
  }

  if target_idx != 0 && !came_from.contains_key(&target_idx) {
    return None;
  }

  let mut transitions = Vec::new();
  let mut idx = target_idx;
  while let Some(&(t, prev)) = came_from.get(&idx) {
    transitions.push(t);
    idx = prev;
  }
  transitions.reverse();

  let states = replay_path(net, from, &transitions);
  Some((states, transitions))
}

pub struct NetProperties {
  pub boundedness: Vec<(ModelPlaceId, petricrab_core::Boundedness)>,
  pub safe: bool,
  pub behavior: NetBehavior,
}

fn to_model_marking(
  net: &petricrab_core::PetriNet,
  reverse_places: &HashMap<petricrab_core::PlaceId, ModelPlaceId>,
  marking: &petricrab_core::Marking,
) -> ModelMarking {
  net
    .place_ids()
    .filter_map(|p| {
      reverse_places
        .get(&p)
        .map(|&mp| (mp, marking.tokens(p) as u32))
    })
    .collect()
}

pub fn analyze(net: &ModelNet, marking: &ModelMarking) -> NetProperties {
  let analysis = to_analysis_net(net, marking);
  let boundedness_report = petricrab_core::boundedness_report(&analysis.net, &analysis.initial);

  let boundedness: Vec<_> = analysis
    .place_map
    .iter()
    .map(|(&model_p, analysis_p)| (model_p, boundedness_report[analysis_p]))
    .collect();
  let safe = boundedness.iter().all(|(_, b)| b.is_safe());
  let precise = boundedness
    .iter()
    .all(|(_, b)| !matches!(b, petricrab_core::Boundedness::Unbounded));

  let liveness_report = if precise {
    petricrab_core::liveness_report(&analysis.net, &analysis.initial)
  } else {
    petricrab_core::liveness_report_covering(&analysis.net, &analysis.initial)
  };

  // Need this direction too: liveness_report's `example` comes back keyed by
  // petricrab_core::TransitionId, and the GUI wants to show it as the editable model's own
  // transitions (so it can look up labels).
  let reverse_transitions: HashMap<petricrab_core::TransitionId, ModelTransitionId> = analysis
    .transition_map
    .iter()
    .map(|(&model_t, &analysis_t)| (analysis_t, model_t))
    .collect();

  let liveness: Vec<_> = analysis
    .transition_map
    .iter()
    .map(|(&model_t, analysis_t)| {
      let report = &liveness_report[analysis_t];
      let example = report
        .example
        .iter()
        .filter_map(|t| reverse_transitions.get(t).copied())
        .collect();
      TransitionLiveness {
        transition: model_t,
        level: report.level,
        example,
      }
    })
    .collect();

  let reverse_places: HashMap<petricrab_core::PlaceId, ModelPlaceId> = analysis
    .place_map
    .iter()
    .map(|(&model_p, &analysis_p)| (analysis_p, model_p))
    .collect();

  let reversible = if precise {
    petricrab_core::is_reversible(&analysis.net, &analysis.initial)
  } else {
    petricrab_core::is_reversible_covering(&analysis.net, &analysis.initial)
  };

  let home_states = if precise {
    petricrab_core::home_states(&analysis.net, &analysis.initial)
      .into_iter()
      .map(|home| to_model_marking(&analysis.net, &reverse_places, &home))
      .collect()
  } else if reversible {
    // Not the full set, but the coverability graph's root is never Ω (see
    // `is_reversible_covering`'s doc), so `initial_marking` itself is always a valid answer.
    vec![marking.clone()]
  } else {
    Vec::new()
  };

  let deadlock_examples: Vec<Vec<ModelTransitionId>> = if precise {
    petricrab_core::deadlocks(&analysis.net, &analysis.initial)
      .into_values()
      .map(|example| {
        example
          .iter()
          .filter_map(|t| reverse_transitions.get(t).copied())
          .collect()
      })
      .collect()
  } else {
    petricrab_core::deadlocks_covering(&analysis.net, &analysis.initial)
      .into_iter()
      .map(|example| {
        example
          .iter()
          .filter_map(|t| reverse_transitions.get(t).copied())
          .collect()
      })
      .collect()
  };
  let deadlocks: Vec<Deadlock> = deadlock_examples
    .into_iter()
    .map(|example| {
      let states = replay_path(net, marking, &example);
      Deadlock { example, states }
    })
    .collect();

  NetProperties {
    boundedness,
    safe,
    behavior: NetBehavior {
      liveness,
      reversible,
      home_states,
      deadlocks,
      precise,
    },
  }
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
