use std::collections::{HashMap, HashSet};

use eframe::egui;

use crate::app::{NodeId, PetriApp};

use super::history::checkpoint;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Align {
  /// Levels the group along whichever axis it's already loosely lined up on (a wider-than-tall
  /// selection becomes a row, a taller-than-wide one a column) AND spaces it evenly along that
  /// axis by `app.align_gap` — same call a "tidy up" tool in a drawing app would make. Unlike
  /// the axis-only variants below, this one also moves nodes along their own spread axis, so it
  /// fixes a bunched-together selection instead of just leaving the clutter in place.
  Auto,
  Left,
  Center,
  Right,
  Top,
  Middle,
  Bottom,
}

/// Aligns every node in `nodes` along `align`. No-op below two nodes — there's nothing to align
/// relative to.
pub(crate) fn align_selected(app: &mut PetriApp, nodes: &HashSet<NodeId>, align: Align) {
  let mut positions: Vec<(NodeId, egui::Pos2)> = nodes
    .iter()
    .filter_map(|&n| app.positions.get(&n).map(|&p| (n, p)))
    .collect();
  if positions.len() < 2 {
    return;
  }
  checkpoint(app);

  let (min_x, max_x) = positions
    .iter()
    .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), (_, p)| {
      (lo.min(p.x), hi.max(p.x))
    });
  let (min_y, max_y) = positions
    .iter()
    .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), (_, p)| {
      (lo.min(p.y), hi.max(p.y))
    });
  let avg_x = positions.iter().map(|(_, p)| p.x).sum::<f32>() / positions.len() as f32;
  let avg_y = positions.iter().map(|(_, p)| p.y).sum::<f32>() / positions.len() as f32;

  if align == Align::Auto {
    let auto_row = (max_x - min_x) >= (max_y - min_y);
    let n = positions.len() as f32;
    // Spacing nodes at a flat `align_gap` regardless of how far apart they already are made
    // Auto feel like it was teleporting an already-tidy selection to an unrelated layout. Only
    // *expand* to `align_gap` when the group is tighter than that (the actually-bunched case);
    // a group that's already spread out further than the gap keeps its existing footprint and
    // just gets its spacing evened out.
    if auto_row {
      positions.sort_by(|(_, a), (_, b)| a.x.total_cmp(&b.x));
      let step = ((max_x - min_x) / (n - 1.0)).max(app.align_gap);
      for (i, (id, _)) in positions.iter().enumerate() {
        app
          .positions
          .insert(*id, egui::pos2(min_x + i as f32 * step, avg_y));
      }
    } else {
      positions.sort_by(|(_, a), (_, b)| a.y.total_cmp(&b.y));
      let step = ((max_y - min_y) / (n - 1.0)).max(app.align_gap);
      for (i, (id, _)) in positions.iter().enumerate() {
        app
          .positions
          .insert(*id, egui::pos2(avg_x, min_y + i as f32 * step));
      }
    }
    return;
  }

  for (n, p) in positions {
    let new_pos = match align {
      Align::Left => egui::pos2(min_x, p.y),
      Align::Center => egui::pos2(avg_x, p.y),
      Align::Right => egui::pos2(max_x, p.y),
      Align::Top => egui::pos2(p.x, min_y),
      Align::Middle => egui::pos2(p.x, avg_y),
      Align::Bottom => egui::pos2(p.x, max_y),
      Align::Auto => unreachable!("handled above"),
    };
    app.positions.insert(n, new_pos);
  }
}

/// Target resting length for a connected pair once the layout settles — also doubles as the
/// Fruchterman-Reingold repulsion constant `k`, per the standard formulation.
const IDEAL_EDGE_LENGTH: f32 = 110.0;
const BEAUTIFY_ITERATIONS: usize = 1500;

/// The "Beautify" toolbar button: relaxes `nodes` (or, with fewer than two selected, the whole
/// net) into a clearer arrangement with a force-directed layout (Fruchterman-Reingold) —
/// connected nodes attract toward a comfortable spacing, every pair of nodes repels so nothing
/// overlaps or crowds. This only ever rewrites `app.positions`; the net itself (arcs, tokens,
/// labels) is read-only here. Starts from the current positions rather than scattering nodes
/// randomly, so a cycle in the net tends to relax into a round, readable loop and a "shortcut"
/// arc settles wherever it stops fighting the rest of the layout, instead of either being forced
/// through a fixed slot on a ring or left wherever it happened to be dropped.
pub(crate) fn beautify(app: &mut PetriApp, nodes: &HashSet<NodeId>) {
  let nodes: HashSet<NodeId> = if nodes.len() >= 2 {
    nodes.clone()
  } else {
    app
      .net
      .place_ids()
      .map(NodeId::Place)
      .chain(app.net.transition_ids().map(NodeId::Transition))
      .collect()
  };

  let mut positions: HashMap<NodeId, egui::Pos2> = nodes
    .iter()
    .filter_map(|&n| app.positions.get(&n).map(|&p| (n, p)))
    .collect();
  // Sorted so the force sums below add up in the same order every run — `HashMap`/`HashSet`
  // iteration order is randomized per process, and floating-point addition isn't associative,
  // so leaving it unsorted made which local minimum the layout settles into nondeterministic
  // (same net, same starting positions, different result depending on the hashing seed).
  let mut ids: Vec<NodeId> = positions.keys().copied().collect();
  ids.sort();
  if ids.len() < 2 {
    return;
  }
  let mut edges = arc_pairs(&app.net, &nodes);
  edges.sort();

  let k = IDEAL_EDGE_LENGTH;
  let mut temperature = k;
  let cooling = temperature / BEAUTIFY_ITERATIONS as f32;

  for _ in 0..BEAUTIFY_ITERATIONS {
    let mut force: HashMap<NodeId, egui::Vec2> = ids.iter().map(|&n| (n, egui::Vec2::ZERO)).collect();

    // Repulsion: every pair pushes apart, strength k^2 / distance — this is what keeps
    // unconnected nodes from crowding each other and pries overlapping ones apart.
    for i in 0..ids.len() {
      for j in (i + 1)..ids.len() {
        let (a, b) = (ids[i], ids[j]);
        let delta = positions[&a] - positions[&b];
        let dist = delta.length();
        // Exactly (or nearly) coincident nodes have no real direction to repel along — a
        // deterministic pseudo-direction from their ids breaks the tie so they still separate
        // instead of the force vector being undefined (NaN from normalizing a zero vector).
        let dir = if dist < 0.01 { jitter_dir(a, b) } else { delta / dist };
        let push = dir * (k * k / dist.max(0.01));
        *force.get_mut(&a).expect("a is in ids") += push;
        *force.get_mut(&b).expect("b is in ids") -= push;
      }
    }

    // Attraction: arcs pull their two ends toward `k` apart, strength distance^2 / k.
    for &(a, b) in &edges {
      let delta = positions[&a] - positions[&b];
      let dist = delta.length().max(0.01);
      let pull = (delta / dist) * (dist * dist / k);
      *force.get_mut(&a).expect("a is in ids") -= pull;
      *force.get_mut(&b).expect("b is in ids") += pull;
    }

    // Apply, capped to the current "temperature" so early large swings settle into small
    // adjustments by the end instead of oscillating forever.
    for &n in &ids {
      let f = force[&n];
      let len = f.length();
      if len > 0.01 {
        positions.insert(n, positions[&n] + f / len * len.min(temperature));
      }
    }
    temperature = (temperature - cooling).max(0.0);
  }

  checkpoint(app);
  for (n, p) in positions {
    app.positions.insert(n, p);
  }
}

/// Undirected edges among `nodes`, from the net's arcs, restricted to arcs that run strictly
/// between two nodes in the set (same convention as `editor::copy_selection`: an arc to
/// something outside the set isn't meaningful here).
fn arc_pairs(net: &crate::model::PetriNet, nodes: &HashSet<NodeId>) -> Vec<(NodeId, NodeId)> {
  let mut edges = Vec::new();
  for &n in nodes {
    if let NodeId::Transition(t) = n {
      for &(p, _) in net.inputs(t) {
        let place = NodeId::Place(p);
        if nodes.contains(&place) {
          edges.push((place, n));
        }
      }
      for &(p, _) in net.outputs(t) {
        let place = NodeId::Place(p);
        if nodes.contains(&place) {
          edges.push((n, place));
        }
      }
    }
  }
  edges
}

/// Deterministic stand-in for a random direction, used only to break ties between exactly
/// coincident nodes during repulsion (see `beautify`) — no `rand` dependency needed for
/// something this small.
fn jitter_dir(a: NodeId, b: NodeId) -> egui::Vec2 {
  use std::hash::{Hash, Hasher};
  let mut hasher = std::collections::hash_map::DefaultHasher::new();
  a.hash(&mut hasher);
  b.hash(&mut hasher);
  let angle = (hasher.finish() % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
  egui::vec2(angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn place_at(app: &mut PetriApp, x: f32, y: f32) -> NodeId {
    let id = app.net.add_place("p");
    let node = NodeId::Place(id);
    app.positions.insert(node, egui::pos2(x, y));
    node
  }

  #[test]
  fn align_left_sets_every_x_to_the_minimum() {
    let mut app = PetriApp::new();
    let a = place_at(&mut app, 0.0, 0.0);
    let b = place_at(&mut app, 100.0, 50.0);
    align_selected(&mut app, &HashSet::from([a, b]), Align::Left);
    assert_eq!(app.positions[&a], egui::pos2(0.0, 0.0));
    assert_eq!(app.positions[&b], egui::pos2(0.0, 50.0)); // y untouched
  }

  #[test]
  fn align_center_sets_every_x_to_the_average() {
    let mut app = PetriApp::new();
    let a = place_at(&mut app, 0.0, 0.0);
    let b = place_at(&mut app, 100.0, 0.0);
    align_selected(&mut app, &HashSet::from([a, b]), Align::Center);
    assert_eq!(app.positions[&a].x, 50.0);
    assert_eq!(app.positions[&b].x, 50.0);
  }

  #[test]
  fn align_top_sets_every_y_to_the_minimum() {
    let mut app = PetriApp::new();
    let a = place_at(&mut app, 0.0, 0.0);
    let b = place_at(&mut app, 50.0, 100.0);
    align_selected(&mut app, &HashSet::from([a, b]), Align::Top);
    assert_eq!(app.positions[&a], egui::pos2(0.0, 0.0));
    assert_eq!(app.positions[&b], egui::pos2(50.0, 0.0)); // x untouched
  }

  #[test]
  fn align_auto_levels_a_wide_group_into_a_row() {
    let mut app = PetriApp::new();
    // Wider than tall: auto should level the y's (turn it into a row), spaced along x.
    let a = place_at(&mut app, 0.0, 0.0);
    let b = place_at(&mut app, 100.0, 20.0);
    align_selected(&mut app, &HashSet::from([a, b]), Align::Auto);
    assert_eq!(app.positions[&a].y, app.positions[&b].y);
    assert_eq!(app.positions[&a].x, 0.0); // leftmost node anchors the row
  }

  #[test]
  fn align_auto_spreads_out_a_bunched_row_instead_of_leaving_it_clumped() {
    let mut app = PetriApp::new();
    // Three nodes almost on top of each other — a naive "level one axis" pass would leave them
    // still overlapping. Auto should space them apart by `align_gap` along x.
    let a = place_at(&mut app, 0.0, 0.0);
    let b = place_at(&mut app, 5.0, 2.0);
    let c = place_at(&mut app, 10.0, -1.0);
    app.align_gap = 100.0;
    align_selected(&mut app, &HashSet::from([a, b, c]), Align::Auto);
    let mut xs = [
      app.positions[&a].x,
      app.positions[&b].x,
      app.positions[&c].x,
    ];
    xs.sort_by(f32::total_cmp);
    assert_eq!(xs, [0.0, 100.0, 200.0]);
  }

  #[test]
  fn align_auto_keeps_footprint_when_already_spread_past_the_gap() {
    let mut app = PetriApp::new();
    // Already spaced 150px apart, well past the 100px gap floor — Auto should just level y
    // in place, not yank everything down to a cramped 100px grid.
    let a = place_at(&mut app, 0.0, 0.0);
    let b = place_at(&mut app, 150.0, 30.0);
    let c = place_at(&mut app, 300.0, -10.0);
    app.align_gap = 100.0;
    align_selected(&mut app, &HashSet::from([a, b, c]), Align::Auto);
    let mut xs = [
      app.positions[&a].x,
      app.positions[&b].x,
      app.positions[&c].x,
    ];
    xs.sort_by(f32::total_cmp);
    assert_eq!(xs, [0.0, 150.0, 300.0]);
  }

  fn link(app: &mut PetriApp, p: crate::model::PlaceId, t: crate::model::TransitionId) {
    app
      .net
      .add_arc_place_to_transition(p, t, crate::model::ArcKind::Consume(1))
      .unwrap();
  }
  fn link_back(app: &mut PetriApp, t: crate::model::TransitionId, p: crate::model::PlaceId) {
    app.net.add_arc_transition_to_place(t, p, 1).unwrap();
  }

  /// True iff segments a-b and c-d properly cross (standard orientation test; pairs sharing an
  /// endpoint are skipped by the caller, since touching at a shared node isn't a crossing).
  fn segments_cross(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2, d: egui::Pos2) -> bool {
    fn orient(p: egui::Pos2, q: egui::Pos2, r: egui::Pos2) -> f32 {
      (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x)
    }
    let (o1, o2) = (orient(a, b, c), orient(a, b, d));
    let (o3, o4) = (orient(c, d, a), orient(c, d, b));
    (o1 * o2 < 0.0) && (o3 * o4 < 0.0)
  }

  fn count_crossings(positions: &HashMap<NodeId, egui::Pos2>, edges: &[(NodeId, NodeId)]) -> usize {
    let mut crossings = 0;
    for i in 0..edges.len() {
      for j in (i + 1)..edges.len() {
        let (a, b) = edges[i];
        let (c, d) = edges[j];
        if a == c || a == d || b == c || b == d {
          continue;
        }
        if segments_cross(positions[&a], positions[&b], positions[&c], positions[&d]) {
          crossings += 1;
        }
      }
    }
    crossings
  }

  #[test]
  fn beautify_separates_exactly_coincident_nodes() {
    let mut app = PetriApp::new();
    let a = place_at(&mut app, 10.0, 10.0);
    let b = place_at(&mut app, 10.0, 10.0);
    beautify(&mut app, &HashSet::from([a, b]));
    let dist = (app.positions[&a] - app.positions[&b]).length();
    assert!(dist > 10.0, "expected the pair to separate, got dist {dist}");
  }

  #[test]
  fn beautify_settles_a_simple_cycle_with_no_overlaps() {
    let mut app = PetriApp::new();
    let p1 = app.net.add_place("p1");
    let t1 = app.net.add_transition("t1");
    let p2 = app.net.add_place("p2");
    let t2 = app.net.add_transition("t2");
    link(&mut app, p1, t1);
    link_back(&mut app, t1, p2);
    link(&mut app, p2, t2);
    link_back(&mut app, t2, p1);
    // All bunched together at the start, like nodes dropped on the same spot.
    let nodes = [NodeId::Place(p1), NodeId::Transition(t1), NodeId::Place(p2), NodeId::Transition(t2)];
    for (i, &n) in nodes.iter().enumerate() {
      app.positions.insert(n, egui::pos2(i as f32, 0.0));
    }

    beautify(&mut app, &nodes.into_iter().collect());

    for i in 0..nodes.len() {
      for j in (i + 1)..nodes.len() {
        let dist = (app.positions[&nodes[i]] - app.positions[&nodes[j]]).length();
        assert!(dist > 20.0, "{:?}-{:?} still overlapping: {dist}", nodes[i], nodes[j]);
      }
    }
  }

  /// Regression test for the shape that broke both earlier layout attempts: an 8-node ring plus
  /// a shortcut transition (t3) directly connecting two ring places. The force-directed relax
  /// shouldn't need any special-casing for this — it should just settle into something that
  /// doesn't cross itself.
  #[test]
  fn beautify_a_ring_with_a_shortcut_has_no_crossings() {
    let mut app = PetriApp::new();
    let p1 = app.net.add_place("p1");
    let p2 = app.net.add_place("p2");
    let p3 = app.net.add_place("p3");
    let p4 = app.net.add_place("p4");
    let t2 = app.net.add_transition("t2");
    let t3 = app.net.add_transition("t3");
    let t4 = app.net.add_transition("t4");
    let t5 = app.net.add_transition("t5");
    let t6 = app.net.add_transition("t6");

    link(&mut app, p1, t2);
    link_back(&mut app, t2, p2);
    link(&mut app, p2, t4);
    link_back(&mut app, t4, p4);
    link(&mut app, p4, t5);
    link_back(&mut app, t5, p3);
    link(&mut app, p3, t6);
    link_back(&mut app, t6, p1);
    link(&mut app, p1, t3);
    link_back(&mut app, t3, p4);

    let nodes: HashSet<NodeId> = [p1, p2, p3, p4]
      .map(NodeId::Place)
      .into_iter()
      .chain([t2, t3, t4, t5, t6].map(NodeId::Transition))
      .collect();
    // Scatter them arbitrarily first — beautify shouldn't depend on a lucky starting layout.
    // Iterated from a sorted Vec, not `nodes.iter()` directly: HashSet iteration order is
    // randomized per process, which would make the scatter (and so the final settled layout)
    // different every run.
    let mut sorted_nodes: Vec<NodeId> = nodes.iter().copied().collect();
    sorted_nodes.sort();
    for (i, &n) in sorted_nodes.iter().enumerate() {
      app.positions.insert(n, egui::pos2((i * 7 % 5) as f32, (i * 3 % 4) as f32));
    }

    beautify(&mut app, &nodes);

    let edges = [
      (NodeId::Place(p1), NodeId::Transition(t2)),
      (NodeId::Transition(t2), NodeId::Place(p2)),
      (NodeId::Place(p2), NodeId::Transition(t4)),
      (NodeId::Transition(t4), NodeId::Place(p4)),
      (NodeId::Place(p4), NodeId::Transition(t5)),
      (NodeId::Transition(t5), NodeId::Place(p3)),
      (NodeId::Place(p3), NodeId::Transition(t6)),
      (NodeId::Transition(t6), NodeId::Place(p1)),
      (NodeId::Place(p1), NodeId::Transition(t3)),
      (NodeId::Transition(t3), NodeId::Place(p4)),
    ];
    assert_eq!(count_crossings(&app.positions, &edges), 0);
  }
}
