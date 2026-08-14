use std::collections::HashSet;

use eframe::egui;

use crate::app::{NodeId, PetriApp, Selection};
use crate::model::ArcKind;

use super::draw::{arc_bow, dist_to_arc, note_rect};

pub(crate) const PLACE_RADIUS: f32 = 24.0;
pub(crate) const T_HALF_W: f32 = 6.0;
pub(crate) const T_HALF_H: f32 = 26.0;
pub(crate) const GRID_SPACING: f32 = 24.0;
const ARC_HIT_DIST: f32 = 6.0;
const ROUNDED_CORNER_SEGMENTS: usize = 8;
const NOTE_RESIZE_HANDLE: f32 = 14.0;

/// All node/arc geometry (positions in `PetriApp::positions`, hit-testing, marquee math) lives
/// in an infinite "world" space. `pan`/`zoom` describe the screen-space view of that world
/// (`screen = world * zoom + pan`); the conversion is applied only at the two boundaries: once
/// when reading pointer input (screen -> world) and once per painter call (world -> screen).
pub(crate) fn to_screen(world: egui::Pos2, pan: egui::Vec2, zoom: f32) -> egui::Pos2 {
  egui::pos2(world.x * zoom + pan.x, world.y * zoom + pan.y)
}

pub(crate) fn to_world(screen: egui::Pos2, pan: egui::Vec2, zoom: f32) -> egui::Pos2 {
  egui::pos2((screen.x - pan.x) / zoom, (screen.y - pan.y) / zoom)
}

/// World-space `pos` rounded to the nearest grid intersection (`GRID_SPACING`, the same step
/// `draw_grid` paints dots at) — used while dragging with Shift held.
pub(crate) fn snap_to_grid(pos: egui::Pos2) -> egui::Pos2 {
  egui::pos2(
    (pos.x / GRID_SPACING).round() * GRID_SPACING,
    (pos.y / GRID_SPACING).round() * GRID_SPACING,
  )
}

#[derive(Clone, Copy)]
pub(crate) enum ArcEnd {
  Arrow,
  DoubleArrow,
  Circle,
}

pub(crate) fn arc_end_for_kind(kind: ArcKind) -> ArcEnd {
  match kind {
    ArcKind::Consume(_) => ArcEnd::Arrow,
    ArcKind::Peek(_) => ArcEnd::DoubleArrow,
    ArcKind::Inhibit(_) => ArcEnd::Circle,
  }
}

pub(crate) fn node_pos(app: &PetriApp, node: NodeId, fallback: egui::Pos2) -> egui::Pos2 {
  app.positions.get(&node).copied().unwrap_or(fallback)
}

/// Degrees clockwise from the default vertical bar. Missing entry means 0°.
pub(crate) fn transition_angle(app: &PetriApp, t: crate::model::TransitionId) -> f32 {
  app.rotation.get(&t).copied().unwrap_or(0.0)
}

/// Normalizes into `[0, 360)`. Drops the entry entirely at exactly 0°.
pub(crate) fn set_rotation(app: &mut PetriApp, t: crate::model::TransitionId, degrees: f32) {
  let normalized = degrees.rem_euclid(360.0);
  if normalized < 0.01 {
    app.rotation.remove(&t);
  } else {
    app.rotation.insert(t, normalized);
  }
}

pub(crate) fn nudge_rotation(app: &mut PetriApp, t: crate::model::TransitionId, delta: f32) {
  set_rotation(app, t, transition_angle(app, t) + delta);
}

/// Rotates `v` clockwise by `degrees` (Y grows downward, matching screen/world space).
fn rotate_vec(v: egui::Vec2, degrees: f32) -> egui::Vec2 {
  if degrees == 0.0 {
    return v;
  }
  let (s, c) = degrees.to_radians().sin_cos();
  egui::vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// Screen-space points approximating a rounded rectangle centered at `pos`, half-extents
/// `half_w`/`half_h` (already zoom-scaled), rotated `angle` degrees clockwise (see
/// `rotate_vec`). Radius is `half_w.min(half_h)` — the same "radius = half the short side"
/// convention the axis-aligned pill already uses — so this matches that exact capsule shape
/// at angle 0 and stays consistent at any other angle. The result is convex, so it paints
/// directly via `egui::Shape::convex_polygon`; egui has no rotated-rounded-rect primitive.
pub(crate) fn rounded_rect_polygon(
  pos: egui::Pos2,
  half_w: f32,
  half_h: f32,
  angle: f32,
) -> Vec<egui::Pos2> {
  let r = half_w.min(half_h);
  let corners = [
    (egui::vec2(half_w - r, -(half_h - r)), -90.0f32, 0.0f32),
    (egui::vec2(half_w - r, half_h - r), 0.0, 90.0),
    (egui::vec2(-(half_w - r), half_h - r), 90.0, 180.0),
    (egui::vec2(-(half_w - r), -(half_h - r)), 180.0, 270.0),
  ];
  let mut points = Vec::with_capacity((ROUNDED_CORNER_SEGMENTS + 1) * 4);
  for (center, start_deg, end_deg) in corners {
    for i in 0..=ROUNDED_CORNER_SEGMENTS {
      let t = i as f32 / ROUNDED_CORNER_SEGMENTS as f32;
      let deg = start_deg + (end_deg - start_deg) * t;
      let local = center + egui::vec2(deg.to_radians().cos(), deg.to_radians().sin()) * r;
      points.push(pos + rotate_vec(local, angle));
    }
  }
  points
}

/// Distance from the node's center to its boundary along `dir` (unit vector, pointing away
/// from the node), in world units. Places are circles (constant radius); transitions are
/// rectangles that may be rotated, so `dir` is expressed in the rectangle's own (unrotated)
/// local frame before applying the usual axis-aligned formula.
pub(crate) fn boundary_margin(app: &PetriApp, node: NodeId, dir: egui::Vec2) -> f32 {
  match node {
    NodeId::Place(_) => PLACE_RADIUS,
    NodeId::Transition(t) => {
      let local = rotate_vec(dir, -transition_angle(app, t));
      let dx = if local.x.abs() > 1e-4 {
        T_HALF_W / local.x.abs()
      } else {
        f32::INFINITY
      };
      let dy = if local.y.abs() > 1e-4 {
        T_HALF_H / local.y.abs()
      } else {
        f32::INFINITY
      };
      dx.min(dy)
    }
  }
}

pub(crate) fn compatible(from: NodeId, to: NodeId) -> bool {
  matches!(
    (from, to),
    (NodeId::Place(_), NodeId::Transition(_)) | (NodeId::Transition(_), NodeId::Place(_))
  )
}

pub(crate) fn dist_to_segment(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
  let ab = b - a;
  let len2 = ab.length_sq();
  if len2 < 1e-6 {
    return (p - a).length();
  }
  let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
  (p - (a + ab * t)).length()
}

/// `pos` is in world space.
pub(crate) fn hit_test(app: &PetriApp, pos: egui::Pos2) -> Option<NodeId> {
  for (&node, &node_pos) in &app.positions {
    let hit = match node {
      NodeId::Place(_) => node_pos.distance(pos) <= PLACE_RADIUS,
      NodeId::Transition(t) => {
        let local = rotate_vec(pos - node_pos, -transition_angle(app, t));
        local.x.abs() <= T_HALF_W && local.y.abs() <= T_HALF_H
      }
    };
    if hit {
      return Some(node);
    }
  }
  None
}

/// `pos` is in world space. Notes live outside `app.positions`/`NodeId` (see `NoteData`), so
/// they get their own small hit-test instead of a branch in `hit_test`.
pub(crate) fn note_hit_test(app: &PetriApp, pos: egui::Pos2) -> Option<crate::app::NoteId> {
  app.notes.iter().find_map(|(id, note)| {
    if note_rect(note).contains(pos) {
      Some(id)
    } else {
      None
    }
  })
}

/// `pos` is in world space. The small square at a note's bottom-right corner used to resize
/// it — checked before `note_hit_test` on drag-start so grabbing the corner resizes instead of
/// moving the whole note.
pub(crate) fn note_resize_hit_test(app: &PetriApp, pos: egui::Pos2) -> Option<crate::app::NoteId> {
  app.notes.iter().find_map(|(id, note)| {
    let corner = note.pos + note.size;
    if (pos - corner).length() <= NOTE_RESIZE_HANDLE {
      Some(id)
    } else {
      None
    }
  })
}

/// `pos` is in world space. Approximates the curved arc as its control-point triangle for
/// hit-testing (cheap and close enough for a ~6px pick radius).
pub(crate) fn hit_test_arc(app: &PetriApp, pos: egui::Pos2) -> Option<Selection> {
  for t in app.net.transition_ids() {
    let Some(t_pos) = app.positions.get(&NodeId::Transition(t)).copied() else {
      continue;
    };
    for &(p, _kind) in app.net.inputs(t) {
      let Some(p_pos) = app.positions.get(&NodeId::Place(p)).copied() else {
        continue;
      };
      let bow = arc_bow(app, p_pos, t_pos, NodeId::Place(p), NodeId::Transition(t));
      if dist_to_arc(pos, p_pos, t_pos, bow) <= ARC_HIT_DIST {
        return Some(Selection::ArcIn(p, t));
      }
    }
    for &(p, _weight) in app.net.outputs(t) {
      let Some(p_pos) = app.positions.get(&NodeId::Place(p)).copied() else {
        continue;
      };
      let bow = arc_bow(app, t_pos, p_pos, NodeId::Transition(t), NodeId::Place(p));
      if dist_to_arc(pos, t_pos, p_pos, bow) <= ARC_HIT_DIST {
        return Some(Selection::ArcOut(t, p));
      }
    }
  }
  None
}

/// `rect` is in world space.
pub(crate) fn nodes_in_rect(app: &PetriApp, rect: egui::Rect) -> HashSet<NodeId> {
  app
    .positions
    .iter()
    .filter(|&(_, &pos)| rect.contains(pos))
    .map(|(&node, _)| node)
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn snap_to_grid_rounds_to_nearest_step() {
    assert_eq!(snap_to_grid(egui::pos2(10.0, 10.0)), egui::pos2(0.0, 0.0));
    assert_eq!(snap_to_grid(egui::pos2(13.0, 40.0)), egui::pos2(24.0, 48.0));
  }
}
