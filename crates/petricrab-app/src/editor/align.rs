use std::collections::HashSet;

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

}
