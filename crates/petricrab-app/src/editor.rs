use std::collections::{HashMap, HashSet};

use crate::model::{ArcKind, PetriNet, PlaceId, TransitionId, fire};
use eframe::egui;

use crate::app::{ContextTarget, EditMode, NodeId, PetriApp, Selection};
use crate::icons;
use crate::theme;

const PLACE_RADIUS: f32 = 24.0;
const T_HALF_W: f32 = 6.0;
const T_HALF_H: f32 = 26.0;
const GRID_SPACING: f32 = 24.0;
const ARC_HIT_DIST: f32 = 6.0;
const HALO_PAD: f32 = 4.0;
const ARC_CURVE_SEGMENTS: usize = 16;
const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 3.0;
const MAX_ARROWHEADS: u32 = 4;
const ROUNDED_CORNER_SEGMENTS: usize = 8;
const INHIBIT_DASH_LEN: f32 = 5.0;
const INHIBIT_GAP_LEN: f32 = 4.0;
const NOTE_MIN_SIZE: egui::Vec2 = egui::vec2(90.0, 54.0);
const NOTE_RESIZE_HANDLE: f32 = 14.0;
/// Extra clearance (beyond a node's own footprint) an arc tries to keep from any node it's not
/// actually connecting, before it's considered "in the way" and worth bowing around.
const ARC_OBSTACLE_PADDING: f32 = 14.0;
/// World-space arc length past which a straight, unobstructed run still gets a gentle bow — a
/// dead-straight long line reads as noise on a busy canvas even with nothing in its way.
const ARC_LONG_THRESHOLD: f32 = 260.0;

/// All node/arc geometry (positions in `PetriApp::positions`, hit-testing, marquee math) lives
/// in an infinite "world" space. `pan`/`zoom` describe the screen-space view of that world
/// (`screen = world * zoom + pan`); the conversion is applied only at the two boundaries: once
/// when reading pointer input (screen -> world) and once per painter call (world -> screen).
fn to_screen(world: egui::Pos2, pan: egui::Vec2, zoom: f32) -> egui::Pos2 {
  egui::pos2(world.x * zoom + pan.x, world.y * zoom + pan.y)
}

fn to_world(screen: egui::Pos2, pan: egui::Vec2, zoom: f32) -> egui::Pos2 {
  egui::pos2((screen.x - pan.x) / zoom, (screen.y - pan.y) / zoom)
}

/// World-space `pos` rounded to the nearest grid intersection (`GRID_SPACING`, the same step
/// `draw_grid` paints dots at) — used while dragging with Shift held.
fn snap_to_grid(pos: egui::Pos2) -> egui::Pos2 {
  egui::pos2(
    (pos.x / GRID_SPACING).round() * GRID_SPACING,
    (pos.y / GRID_SPACING).round() * GRID_SPACING,
  )
}

#[derive(Clone, Copy)]
enum ArcEnd {
  Arrow,
  DoubleArrow,
  Circle,
}

fn arc_end_for_kind(kind: ArcKind) -> ArcEnd {
  match kind {
    ArcKind::Consume(_) => ArcEnd::Arrow,
    ArcKind::Peek(_) => ArcEnd::DoubleArrow,
    ArcKind::Inhibit(_) => ArcEnd::Circle,
  }
}

fn node_pos(app: &PetriApp, node: NodeId, fallback: egui::Pos2) -> egui::Pos2 {
  app.positions.get(&node).copied().unwrap_or(fallback)
}

/// Degrees clockwise from the default vertical bar. Missing entry means 0°.
fn transition_angle(app: &PetriApp, t: crate::model::TransitionId) -> f32 {
  app.rotation.get(&t).copied().unwrap_or(0.0)
}

/// Normalizes into `[0, 360)`. Drops the entry entirely at exactly 0°.
fn set_rotation(app: &mut PetriApp, t: crate::model::TransitionId, degrees: f32) {
  let normalized = degrees.rem_euclid(360.0);
  if normalized < 0.01 {
    app.rotation.remove(&t);
  } else {
    app.rotation.insert(t, normalized);
  }
}

fn nudge_rotation(app: &mut PetriApp, t: crate::model::TransitionId, delta: f32) {
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
fn rounded_rect_polygon(pos: egui::Pos2, half_w: f32, half_h: f32, angle: f32) -> Vec<egui::Pos2> {
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
fn boundary_margin(app: &PetriApp, node: NodeId, dir: egui::Vec2) -> f32 {
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

fn compatible(from: NodeId, to: NodeId) -> bool {
  matches!(
    (from, to),
    (NodeId::Place(_), NodeId::Transition(_)) | (NodeId::Transition(_), NodeId::Place(_))
  )
}

fn dist_to_segment(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
  let ab = b - a;
  let len2 = ab.length_sq();
  if len2 < 1e-6 {
    return (p - a).length();
  }
  let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
  (p - (a + ab * t)).length()
}

/// `pos` is in world space.
fn hit_test(app: &PetriApp, pos: egui::Pos2) -> Option<NodeId> {
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
fn note_hit_test(app: &PetriApp, pos: egui::Pos2) -> Option<crate::app::NoteId> {
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
fn note_resize_hit_test(app: &PetriApp, pos: egui::Pos2) -> Option<crate::app::NoteId> {
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
fn hit_test_arc(app: &PetriApp, pos: egui::Pos2) -> Option<Selection> {
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
fn nodes_in_rect(app: &PetriApp, rect: egui::Rect) -> HashSet<NodeId> {
  app
    .positions
    .iter()
    .filter(|&(_, &pos)| rect.contains(pos))
    .map(|(&node, _)| node)
    .collect()
}

/// A dot at every grid intersection instead of full lines.
fn draw_grid(painter: &egui::Painter, rect: egui::Rect, pan: egui::Vec2, zoom: f32) {
  let c = theme::text();
  let dot = egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 38);
  let spacing = GRID_SPACING * zoom;
  if spacing < 4.0 {
    return;
  }
  let radius = 1.1;

  let mut x = (((rect.left() - pan.x) / spacing).floor() * spacing) + pan.x;
  while x < rect.right() {
    let mut y = (((rect.top() - pan.y) / spacing).floor() * spacing) + pan.y;
    while y < rect.bottom() {
      painter.circle_filled(egui::pos2(x, y), radius, dot);
      y += spacing;
    }
    x += spacing;
  }
}

/// World-space quadratic-bezier control point for the arc between two world-space centers.
/// Straight by default; `bow` (world units, signed — which side to bow toward) comes from
/// `arc_bow`, and moves the control point off the midpoint perpendicular to the line.
fn arc_control_point(a: egui::Pos2, b: egui::Pos2, bow: f32) -> egui::Pos2 {
  let delta = b - a;
  let len = delta.length();
  if len < 1.0 || bow.abs() < 0.5 {
    return a + delta * 0.5;
  }
  let dir = delta / len;
  let normal = egui::vec2(-dir.y, dir.x);
  a + delta * 0.5 + normal * bow
}

/// Whether both a place->transition and transition->place arc exist between this exact pair.
fn is_reciprocal(
  net: &crate::model::PetriNet,
  p: crate::model::PlaceId,
  t: crate::model::TransitionId,
) -> bool {
  net.inputs(t).iter().any(|&(place, _)| place == p)
    && net.outputs(t).iter().any(|&(place, _)| place == p)
}

/// How much (and which way) to bow an arc's control point off the straight line between `from`
/// and `to` (world-space node centers). In order: a straight run that would otherwise cut through
/// (or too close to) some unrelated node bows away from the worst offender; a reciprocal
/// place<->transition pair (arcs running both ways) gets a small fixed bow so the two separate
/// instead of overlapping; a long run with nothing in the way still gets a gentle bow, since a
/// dead-straight long line reads as noise on a busy canvas; anything else stays straight.
fn arc_bow(
  app: &PetriApp,
  from: egui::Pos2,
  to: egui::Pos2,
  from_node: NodeId,
  to_node: NodeId,
) -> f32 {
  let delta = to - from;
  let len = delta.length();
  if len < 1.0 {
    return 0.0;
  }
  let dir = delta / len;
  let normal = egui::vec2(-dir.y, dir.x);

  let obstruction = app
    .positions
    .iter()
    .filter(|&(&node, _)| node != from_node && node != to_node)
    .filter_map(|(&node, &pos)| {
      let radius = match node {
        NodeId::Place(_) => PLACE_RADIUS,
        NodeId::Transition(_) => T_HALF_H,
      } + ARC_OBSTACLE_PADDING;
      let penetration = radius - dist_to_segment(pos, from, to);
      (penetration > 0.0).then_some((penetration, (pos - from).dot(normal).signum()))
    })
    .max_by(|a, b| a.0.total_cmp(&b.0));

  if let Some((penetration, side)) = obstruction {
    let side = if side == 0.0 { 1.0 } else { side };
    return (-side * (penetration * 2.0 + 10.0)).clamp(-len * 0.4, len * 0.4);
  }

  let reciprocal = match (from_node, to_node) {
    (NodeId::Place(p), NodeId::Transition(t)) | (NodeId::Transition(t), NodeId::Place(p)) => {
      is_reciprocal(&app.net, p, t)
    }
    _ => false,
  };
  if reciprocal {
    return (len * 0.15).min(36.0);
  }

  if len > ARC_LONG_THRESHOLD {
    return (len * 0.1).min(28.0);
  }

  0.0
}

/// World-space distance from `p` to the curved arc between world-space centers `a`/`b`.
fn dist_to_arc(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2, bow: f32) -> f32 {
  let control = arc_control_point(a, b, bow);
  let mut prev = a;
  let mut best = f32::INFINITY;
  for i in 1..=ARC_CURVE_SEGMENTS {
    let t = i as f32 / ARC_CURVE_SEGMENTS as f32;
    let point = quad_bezier(a, control, b, t);
    best = best.min(dist_to_segment(p, prev, point));
    prev = point;
  }
  best
}

fn quad_bezier(a: egui::Pos2, control: egui::Pos2, b: egui::Pos2, t: f32) -> egui::Pos2 {
  let mt = 1.0 - t;
  let x = mt * mt * a.x + 2.0 * mt * t * control.x + t * t * b.x;
  let y = mt * mt * a.y + 2.0 * mt * t * control.y + t * t * b.y;
  egui::pos2(x, y)
}

/// `from`/`to` are world-space node centers; this draws a gently curved connector between their
/// (world-space) boundaries, converting to screen space only at the final painter calls so the
/// curve, arrowhead and stroke width all scale correctly with `zoom`. `weight` (arc multiplicity)
/// is shown by repeating the end-cap mark instead of a numeric label, so it reads at a glance.
fn draw_arc(
  app: &PetriApp,
  painter: &egui::Painter,
  from: egui::Pos2,
  to: egui::Pos2,
  from_node: NodeId,
  to_node: NodeId,
  end: ArcEnd,
  weight: u32,
  stroke: egui::Stroke,
  pan: egui::Vec2,
  zoom: f32,
  background: egui::Color32,
) {
  let delta = to - from;
  if delta.length() < 1.0 {
    return;
  }
  let control = arc_control_point(from, to, arc_bow(app, from, to, from_node, to_node));

  // Trim the curve to the node boundaries using the direction at each endpoint (tangent for
  // the transition end, straight line for the start; close enough for a subtle bow).
  let start_dir = (control - from).normalized();
  let end_dir = (to - control).normalized();
  let from_edge = from + start_dir * boundary_margin(app, from_node, start_dir);
  let to_edge = to - end_dir * boundary_margin(app, to_node, -end_dir);

  let ts = |p: egui::Pos2| to_screen(p, pan, zoom);
  let stroke = egui::Stroke::new(stroke.width * zoom, stroke.color);

  let points: Vec<egui::Pos2> = (0..=ARC_CURVE_SEGMENTS)
    .map(|i| {
      let t = i as f32 / ARC_CURVE_SEGMENTS as f32;
      ts(quad_bezier(from_edge, control, to_edge, t))
    })
    .collect();
  if let ArcEnd::Circle = end {
    // Inhibit arcs read as "......o" in the classic notation — dash the line, keep the
    // circle end-cap below solid.
    let mut dashes = Vec::new();
    egui::Shape::dashed_line_many(
      &points,
      stroke,
      INHIBIT_DASH_LEN * zoom,
      INHIBIT_GAP_LEN * zoom,
      &mut dashes,
    );
    painter.extend(dashes);
  } else {
    painter.add(egui::Shape::line(points, stroke));
  }

  let dir = end_dir;

  // A touch longer/narrower than a stock triangle so it reads as a sleek chevron instead of a
  // blunt flag, at both toolbar-icon and zoomed-out scale.
  let chevron = |point: egui::Pos2, dir: egui::Vec2| {
    let back = point - dir * 11.0;
    let n = egui::vec2(-dir.y, dir.x);
    painter.add(egui::Shape::convex_polygon(
      vec![ts(point), ts(back + n * 3.4), ts(back - n * 3.4)],
      stroke.color,
      egui::Stroke::NONE,
    ));
  };

  match end {
    ArcEnd::DoubleArrow => {
      // Peek/test arcs (`<-->`) are bidirectional by nature (reading, not consuming), so they
      // get a chevron at each end pointing outward, rather than one arrowhead at the target.
      // ponytail: ignores `weight`/reps here — the arc-kind selector already caps Peek's
      // weight at 1, so there's no multi-rep stacking case to handle; extend to both ends if
      // that UI cap is ever lifted.
      chevron(to_edge, end_dir);
      chevron(from_edge, -start_dir);
    }
    _ => {
      let reps = weight.clamp(1, MAX_ARROWHEADS);
      for i in 0..reps {
        let tip = to_edge - dir * (i as f32 * 7.0);
        match end {
          ArcEnd::Arrow => chevron(tip, dir),
          ArcEnd::Circle => {
            // Hollow ring, not a filled dot: matches the classic inhibitor-arc notation (a
            // filled circle would read as a token/peek marker instead) and keeps the endpoint
            // light on a busy canvas.
            let center = tip - dir * 4.5;
            painter.circle_filled(ts(center), 4.5 * zoom, background);
            painter.circle_stroke(
              ts(center),
              4.5 * zoom,
              egui::Stroke::new(stroke.width, stroke.color),
            );
          }
          ArcEnd::DoubleArrow => unreachable!(),
        }
      }
    }
  }
}

/// Near-black or near-white, whichever reads clearly on top of `bg` — used for the token dots,
/// since a place's fill can now be any custom color the user picked, not just the theme
/// default, and a fixed token color would go invisible against a similarly-toned fill.
fn contrasting_on(bg: egui::Color32) -> egui::Color32 {
  let luminance = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
  if luminance > 140.0 {
    egui::Color32::from_rgb(20, 20, 22)
  } else {
    egui::Color32::from_rgb(245, 245, 246)
  }
}

/// Draws token count as dots (classic Petri-net notation) for small counts, falling back to a
/// number once dots would get too crowded to read at a glance. `center` is in screen space.
fn draw_tokens(
  painter: &egui::Painter,
  center: egui::Pos2,
  tokens: u32,
  zoom: f32,
  color: egui::Color32,
) {
  let d = |dx: f32, dy: f32| egui::vec2(dx, dy) * zoom;
  match tokens {
    0 => {}
    1 => {
      painter.circle_filled(center, 4.0 * zoom, color);
    }
    2 => {
      painter.circle_filled(center + d(-6.0, 0.0), 3.5 * zoom, color);
      painter.circle_filled(center + d(6.0, 0.0), 3.5 * zoom, color);
    }
    3 => {
      painter.circle_filled(center + d(0.0, -6.0), 3.5 * zoom, color);
      painter.circle_filled(center + d(-6.0, 5.0), 3.5 * zoom, color);
      painter.circle_filled(center + d(6.0, 5.0), 3.5 * zoom, color);
    }
    n => {
      painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        n.to_string(),
        egui::FontId::proportional(14.0 * zoom),
        color,
      );
    }
  };
}

/// `pos` is in screen space.
fn draw_selection_halo(
  app: &PetriApp,
  painter: &egui::Painter,
  node: NodeId,
  pos: egui::Pos2,
  zoom: f32,
  accent: egui::Color32,
) {
  let stroke = egui::Stroke::new(1.6 * zoom, accent);
  match node {
    NodeId::Place(_) => {
      painter.circle_stroke(pos, (PLACE_RADIUS + HALO_PAD) * zoom, stroke);
    }
    NodeId::Transition(t) if transition_angle(app, t) == 0.0 => {
      let r = egui::Rect::from_center_size(
        pos,
        egui::vec2((T_HALF_W + HALO_PAD) * 2.0, (T_HALF_H + HALO_PAD) * 2.0) * zoom,
      );
      let radius = (T_HALF_W + HALO_PAD) * zoom;
      painter.rect_stroke(r, radius, stroke, egui::StrokeKind::Outside);
    }
    NodeId::Transition(t) => {
      let points = rounded_rect_polygon(
        pos,
        (T_HALF_W + HALO_PAD) * zoom,
        (T_HALF_H + HALO_PAD) * zoom,
        transition_angle(app, t),
      );
      painter.add(egui::Shape::convex_polygon(
        points,
        egui::Color32::TRANSPARENT,
        stroke,
      ));
    }
  }
}

fn draw_marquee(painter: &egui::Painter, a: egui::Pos2, b: egui::Pos2, accent: egui::Color32) {
  let rect = egui::Rect::from_two_pos(a, b);
  let fill = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 28);
  painter.rect_filled(rect, 2.0, fill);
  painter.rect_stroke(
    rect,
    2.0,
    egui::Stroke::new(1.0, accent),
    egui::StrokeKind::Outside,
  );
}

fn draw_net(
  app: &PetriApp,
  painter: &egui::Painter,
  fallback: egui::Pos2,
  pan: egui::Vec2,
  zoom: f32,
  visuals: &egui::Visuals,
) {
  let s = |p: egui::Pos2| to_screen(p, pan, zoom);

  let marking = app.net.marking();
  let enabled: HashSet<_> = fire::enabled_transitions(&app.net, &marking)
    .into_iter()
    .collect();
  // Dimmer and thinner than the nodes themselves, so arcs read as connective tissue instead of
  // competing with the crisp white node outlines for attention.
  let arc_stroke = egui::Stroke::new(1.2, theme::text_weak());
  let accent = visuals.selection.bg_fill;

  // While replaying a route, everything not on it fades out so the recorded path reads as the
  // highlight against real context, instead of a same-weight diagram.
  let route = app.route_modal.as_ref();
  let dim = |c: egui::Color32| c.gamma_multiply(0.28);
  let place_dimmed =
    |p: crate::model::PlaceId| route.is_some_and(|r| !r.route_places().contains(&p));
  let transition_dimmed =
    |t: crate::model::TransitionId| route.is_some_and(|r| !r.route_transitions().contains(&t));
  let current_t = route.and_then(|r| r.current_transition());
  let visited_transitions = route.map(|r| r.visited_transitions()).unwrap_or_default();
  // The transition about to fire: inputs (what it's about to consume) in orange, outputs (what
  // it's about to produce) in cyan, thicker than every other arc. Transitions already fired on
  // the way here stay green.
  let route_in_stroke = egui::Stroke::new(2.4, egui::Color32::from_rgb(237, 137, 54));
  let route_out_stroke = egui::Stroke::new(2.4, egui::Color32::from_rgb(94, 202, 232));
  let visited_stroke = egui::Stroke::new(1.8, theme::success());

  // Arcs (under nodes).
  for t in app.net.transition_ids() {
    let t_pos = node_pos(app, NodeId::Transition(t), fallback);
    let t_dim = transition_dimmed(t);
    let is_current = Some(t) == current_t;
    for &(p, kind) in app.net.inputs(t) {
      let p_pos = node_pos(app, NodeId::Place(p), fallback);
      let stroke = if is_current {
        route_in_stroke
      } else if visited_transitions.contains(&t) {
        visited_stroke
      } else if t_dim || place_dimmed(p) {
        egui::Stroke::new(arc_stroke.width, dim(arc_stroke.color))
      } else {
        arc_stroke
      };
      draw_arc(
        app,
        painter,
        p_pos,
        t_pos,
        NodeId::Place(p),
        NodeId::Transition(t),
        arc_end_for_kind(kind),
        kind.weight(),
        stroke,
        pan,
        zoom,
        visuals.extreme_bg_color,
      );
    }
    for &(p, weight) in app.net.outputs(t) {
      let p_pos = node_pos(app, NodeId::Place(p), fallback);
      let stroke = if is_current {
        route_out_stroke
      } else if visited_transitions.contains(&t) {
        visited_stroke
      } else if t_dim || place_dimmed(p) {
        egui::Stroke::new(arc_stroke.width, dim(arc_stroke.color))
      } else {
        arc_stroke
      };
      draw_arc(
        app,
        painter,
        t_pos,
        p_pos,
        NodeId::Transition(t),
        NodeId::Place(p),
        ArcEnd::Arrow,
        weight,
        stroke,
        pan,
        zoom,
        visuals.extreme_bg_color,
      );
    }
  }

  // Selection halos (under the nodes they highlight, over the arcs).
  if let Selection::Nodes(nodes) = &app.selection {
    for &node in nodes {
      draw_selection_halo(
        app,
        painter,
        node,
        s(node_pos(app, node, fallback)),
        zoom,
        accent,
      );
    }
  }

  // The transition about to fire in a route replay gets the same halo treatment, so it stands
  // out from the rest of the (already brighter-than-dimmed) route.
  if let Some(t) = route.and_then(|r| r.current_transition()) {
    draw_selection_halo(
      app,
      painter,
      NodeId::Transition(t),
      s(node_pos(app, NodeId::Transition(t), fallback)),
      zoom,
      theme::accent(),
    );
  }

  // Places: same fill as the canvas itself, just a thin crisp ring. Token dots are the only
  // solid white in the shape.
  for p in app.net.place_ids() {
    let pos = s(node_pos(app, NodeId::Place(p), fallback));
    let dimmed = place_dimmed(p);
    let fill = app.colors.get(&p).copied().unwrap_or(theme::ink());
    painter.circle_filled(pos, PLACE_RADIUS * zoom, fill);
    let ring = if dimmed {
      dim(theme::text())
    } else {
      theme::text()
    };
    painter.circle_stroke(
      pos,
      PLACE_RADIUS * zoom,
      egui::Stroke::new(1.4 * zoom, ring),
    );
    draw_tokens(painter, pos, app.net.tokens(p), zoom, contrasting_on(fill));
    let label_color = if dimmed {
      dim(visuals.weak_text_color())
    } else {
      visuals.weak_text_color()
    };
    painter.text(
      pos + egui::vec2(0.0, (PLACE_RADIUS + 12.0) * zoom),
      egui::Align2::CENTER_CENTER,
      app.net.place_label(p),
      egui::FontId::proportional(12.0 * zoom),
      label_color,
    );
  }

  // Axis-aligned bars use the fast rounded-rect path; a rotated transition draws as an explicit
  // polygon instead, since egui has no rotated-rounded-rect primitive.
  for t in app.net.transition_ids() {
    let pos = s(node_pos(app, NodeId::Transition(t), fallback));
    let angle = transition_angle(app, t);
    let dimmed = transition_dimmed(t);
    let fill = if dimmed {
      dim(theme::text_weak())
    } else if enabled.contains(&t) {
      theme::success()
    } else {
      theme::text_weak()
    };
    let label_y = if angle == 0.0 {
      let r = egui::Rect::from_center_size(pos, egui::vec2(T_HALF_W * 2.0, T_HALF_H * 2.0) * zoom);
      // A radius of half the short side turns the rounded rect into a full capsule/pill.
      painter.rect_filled(r, T_HALF_W * zoom, fill);
      r.bottom()
    } else {
      let points = rounded_rect_polygon(pos, T_HALF_W * zoom, T_HALF_H * zoom, angle);
      painter.add(egui::Shape::convex_polygon(
        points.clone(),
        fill,
        egui::Stroke::NONE,
      ));
      points.iter().map(|c| c.y).fold(f32::MIN, f32::max)
    };
    let label_color = if dimmed {
      dim(visuals.weak_text_color())
    } else {
      visuals.weak_text_color()
    };
    painter.text(
      egui::pos2(pos.x, label_y + 12.0 * zoom),
      egui::Align2::CENTER_CENTER,
      app.net.transition_label(t),
      egui::FontId::proportional(12.0 * zoom),
      label_color,
    );
  }

  // Selected arc, drawn last so it reads clearly on top of everything.
  match &app.selection {
    Selection::ArcIn(p, t) => {
      let (p, t) = (*p, *t);
      let p_pos = node_pos(app, NodeId::Place(p), fallback);
      let t_pos = node_pos(app, NodeId::Transition(t), fallback);
      let kind = app
        .net
        .inputs(t)
        .iter()
        .find(|(place, _)| *place == p)
        .map(|&(_, k)| k);
      if let Some(kind) = kind {
        draw_arc(
          app,
          painter,
          p_pos,
          t_pos,
          NodeId::Place(p),
          NodeId::Transition(t),
          arc_end_for_kind(kind),
          kind.weight(),
          egui::Stroke::new(3.0, accent),
          pan,
          zoom,
          visuals.extreme_bg_color,
        );
      }
    }
    Selection::ArcOut(t, p) => {
      let (t, p) = (*t, *p);
      let t_pos = node_pos(app, NodeId::Transition(t), fallback);
      let p_pos = node_pos(app, NodeId::Place(p), fallback);
      let weight = app
        .net
        .outputs(t)
        .iter()
        .find(|(place, _)| *place == p)
        .map(|&(_, w)| w)
        .unwrap_or(1);
      draw_arc(
        app,
        painter,
        t_pos,
        p_pos,
        NodeId::Transition(t),
        NodeId::Place(p),
        ArcEnd::Arrow,
        weight,
        egui::Stroke::new(3.0, accent),
        pan,
        zoom,
        visuals.extreme_bg_color,
      );
    }
    _ => {}
  }
}

/// World-space rect for a note, mirrors `note_hit_test`/`note_resize_hit_test`.
fn note_rect(note: &crate::app::NoteData) -> egui::Rect {
  egui::Rect::from_min_size(note.pos, note.size)
}

/// Free-form text annotations — a small card per note. The selected one's text is edited live
/// via an actual `TextEdit` placed on top (see `note_edit_overlay`), so its static text is
/// skipped here to avoid drawing under the widget; every other note gets wrapped, clipped
/// static text painted directly (cheaper than a widget for something you're not touching).
fn draw_notes(
  app: &PetriApp,
  painter: &egui::Painter,
  pan: egui::Vec2,
  zoom: f32,
  visuals: &egui::Visuals,
) {
  for (id, note) in app.notes.iter() {
    let rect = egui::Rect::from_min_size(to_screen(note.pos, pan, zoom), note.size * zoom);
    let selected = app.selection == Selection::Note(id);
    painter.rect_filled(
      rect,
      6.0 * zoom,
      note.color.unwrap_or(theme::surface_raised()),
    );
    painter.rect_stroke(
      rect,
      6.0 * zoom,
      egui::Stroke::new(
        if selected { 2.0 } else { 1.0 } * zoom,
        if selected {
          visuals.selection.bg_fill
        } else {
          theme::line_strong()
        },
      ),
      egui::StrokeKind::Outside,
    );

    if app.editing_note != Some(id) {
      let text_rect = rect.shrink(8.0 * zoom);
      let (display, color): (&str, egui::Color32) = if note.text.is_empty() {
        ("Nota vacía…", theme::text_weak())
      } else {
        (note.text.as_str(), theme::text())
      };
      let galley = painter.layout(
        display.to_string(),
        egui::FontId::monospace(11.0 * zoom),
        color,
        text_rect.width(),
      );
      painter
        .with_clip_rect(text_rect)
        .galley(text_rect.left_top(), galley, color);
    }

    // Resize handle: a small corner glyph, only worth showing (and interacting with) once
    // the note is actually selected — otherwise it's just noise on every note at once.
    if selected {
      let handle_size = egui::vec2(10.0, 10.0) * zoom;
      let handle = egui::Rect::from_min_size(rect.max - handle_size, handle_size);
      painter.line_segment(
        [
          egui::pos2(handle.left(), handle.bottom()),
          egui::pos2(handle.right(), handle.top()),
        ],
        egui::Stroke::new(1.5 * zoom, theme::text_weak()),
      );
    }
  }
}

/// Places an actual editable `TextEdit` over the note being edited, if any — the only way to
/// type directly on the canvas instead of only through the Selection tab (`painter.text` can't
/// be interactive). Called after `draw_notes` so it visually sits on top. A single click only
/// selects a note (see `handle_click`); this only shows once `app.editing_note` says so.
fn note_edit_overlay(app: &mut PetriApp, ui: &mut egui::Ui, pan: egui::Vec2, zoom: f32) {
  let Some(id) = app.editing_note else {
    return;
  };
  let Some(note) = app.notes.get_mut(id) else {
    return;
  };
  let rect =
    egui::Rect::from_min_size(to_screen(note.pos, pan, zoom), note.size * zoom).shrink(8.0 * zoom);
  ui.put(
    rect,
    egui::TextEdit::multiline(&mut note.text)
      .frame(egui::Frame::NONE)
      .font(egui::FontId::monospace(11.0 * zoom))
      .desired_width(rect.width())
      .hint_text("Escribí lo que quieras…"),
  );
}

/// `mouse`/`fallback` are in world space.
fn draw_connect_preview(
  app: &PetriApp,
  painter: &egui::Painter,
  from: NodeId,
  mouse: egui::Pos2,
  fallback: egui::Pos2,
  pan: egui::Vec2,
  zoom: f32,
  visuals: &egui::Visuals,
) {
  let s = |p: egui::Pos2| to_screen(p, pan, zoom);
  let from_pos = node_pos(app, from, fallback);
  let hovered = hit_test(app, mouse).filter(|&n| n != from && compatible(from, n));

  let base = visuals.text_color();
  let dimmed = egui::Stroke::new(
    2.0,
    egui::Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 100),
  );

  match hovered {
    Some(target) => {
      let target_pos = node_pos(app, target, fallback);
      draw_selection_halo(
        app,
        painter,
        target,
        s(target_pos),
        zoom,
        visuals.selection.bg_fill,
      );
      draw_arc(
        app,
        painter,
        from_pos,
        target_pos,
        from,
        target,
        ArcEnd::Arrow,
        1,
        dimmed,
        pan,
        zoom,
        visuals.extreme_bg_color,
      );
    }
    None => {
      // No boundary_margin on the free end: it's just the mouse position, not a node.
      let delta = mouse - from_pos;
      if delta.length() >= 1.0 {
        let dir = delta.normalized();
        let from_edge = from_pos + dir * boundary_margin(app, from, dir);
        let stroke = egui::Stroke::new(dimmed.width * zoom, dimmed.color);
        painter.line_segment([s(from_edge), s(mouse)], stroke);
        painter.circle_filled(s(mouse), 3.0 * zoom, dimmed.color);
      }
    }
  }
}

/// Fires `t` and records the pre-fire marking so the simulator can step back through it. Used
/// by both the simulate panel's buttons and the canvas token-game, so stepping back undoes
/// whichever one you used to fire.
fn fire_step(app: &mut PetriApp, t: crate::model::TransitionId) {
  app.sim_history.push(app.net.marking());
  app.sim_future.clear();
  let _ = fire::fire(&mut app.net, t);
}

fn step_back(app: &mut PetriApp) {
  if let Some(prev) = app.sim_history.pop() {
    app.sim_future.push(app.net.marking());
    app.net.set_marking(&prev);
  }
}

fn step_forward(app: &mut PetriApp) {
  if let Some(next) = app.sim_future.pop() {
    app.sim_history.push(app.net.marking());
    app.net.set_marking(&next);
  }
}

fn reset_sim(app: &mut PetriApp) {
  if let Some(initial) = app.sim_initial.clone() {
    app.net.set_marking(&initial);
    app.sim_history.clear();
    app.sim_future.clear();
  }
}

/// Opening the simulate panel snapshots the current marking as the "Reset" target and starts a
/// fresh undo/redo history for this session.
fn toggle_simulate(app: &mut PetriApp) {
  app.simulate_open = !app.simulate_open;
  if app.simulate_open {
    app.sim_initial = Some(app.net.marking());
    app.sim_history.clear();
    app.sim_future.clear();
  }
}

fn connect(app: &mut PetriApp, from: NodeId, to: NodeId) {
  if !compatible(from, to) {
    return;
  }
  checkpoint(app);
  let _ = match (from, to) {
    (NodeId::Place(p), NodeId::Transition(t)) => {
      app
        .net
        .add_arc_place_to_transition(p, t, ArcKind::Consume(1))
    }
    (NodeId::Transition(t), NodeId::Place(p)) => app.net.add_arc_transition_to_place(t, p, 1),
    _ => return,
  };
}

fn delete_selected(app: &mut PetriApp) {
  if matches!(app.selection, Selection::None) {
    return;
  }
  checkpoint(app);
  match std::mem::take(&mut app.selection) {
    Selection::Nodes(nodes) => {
      for node in nodes {
        match node {
          NodeId::Place(p) => {
            app.net.remove_place(p);
            app.positions.remove(&NodeId::Place(p));
          }
          NodeId::Transition(t) => {
            app.net.remove_transition(t);
            app.positions.remove(&NodeId::Transition(t));
          }
        }
      }
    }
    Selection::ArcIn(p, t) => app.net.remove_arc_place_to_transition(p, t),
    Selection::ArcOut(t, p) => app.net.remove_arc_transition_to_place(t, p),
    Selection::Note(id) => {
      app.notes.remove(id);
    }
    Selection::None => {}
  }
}

/// Everything Ctrl+Z should be able to bring back: the net itself, node positions, transition
/// rotations, place colors and notes. `undo_stack`/`redo_stack` on `PetriApp` hold these.
pub(crate) struct Snapshot {
  net: PetriNet,
  positions: HashMap<NodeId, egui::Pos2>,
  rotation: HashMap<TransitionId, f32>,
  colors: HashMap<PlaceId, egui::Color32>,
  notes: slotmap::SlotMap<crate::app::NoteId, crate::app::NoteData>,
}

impl Snapshot {
  fn capture(app: &PetriApp) -> Self {
    Self {
      net: app.net.clone(),
      positions: app.positions.clone(),
      rotation: app.rotation.clone(),
      colors: app.colors.clone(),
      notes: app.notes.clone(),
    }
  }

  fn restore(self, app: &mut PetriApp) {
    app.net = self.net;
    app.positions = self.positions;
    app.rotation = self.rotation;
    app.colors = self.colors;
    app.notes = self.notes;
    app.selection = Selection::None;
    app.selection_focus = None;
  }
}

const MAX_UNDO_STEPS: usize = 100;

/// Saves an undo point capturing the state *before* the mutation that's about to happen, and
/// drops the redo stack (a fresh edit invalidates whatever redo history there was). Called at
/// the start of every action that changes the net, positions, rotation, colors or notes — plain
/// text edits (labels, note text) are the deliberate exception, see the call sites, since
/// checkpointing every keystroke would make one undo step per character typed.
fn checkpoint(app: &mut PetriApp) {
  app.redo_stack.clear();
  app.undo_stack.push(Snapshot::capture(app));
  if app.undo_stack.len() > MAX_UNDO_STEPS {
    app.undo_stack.remove(0);
  }
}

fn undo(app: &mut PetriApp) {
  let Some(prev) = app.undo_stack.pop() else {
    return;
  };
  app.redo_stack.push(Snapshot::capture(app));
  prev.restore(app);
}

fn redo(app: &mut PetriApp) {
  let Some(next) = app.redo_stack.pop() else {
    return;
  };
  app.undo_stack.push(Snapshot::capture(app));
  next.restore(app);
}

#[derive(Clone)]
enum ClipboardNode {
  Place {
    label: String,
    tokens: u32,
    color: Option<egui::Color32>,
  },
  Transition {
    label: String,
    rotation: Option<f32>,
  },
}

/// A copied selection, ready to paste: the nodes themselves plus the arcs that ran strictly
/// between two copied nodes. An arc to a node outside the selection is dropped — pasting a
/// partial subgraph can't recreate a connection to something that wasn't copied. Nodes keep
/// their original `NodeId` here only so `paste_clipboard` can remap arc endpoints to the freshly
/// created ids; those ids are otherwise meaningless once copied.
#[derive(Clone)]
pub(crate) struct Clipboard {
  nodes: Vec<(NodeId, ClipboardNode, egui::Pos2)>,
  arcs_in: Vec<(NodeId, NodeId, ArcKind)>,
  arcs_out: Vec<(NodeId, NodeId, u32)>,
}

fn copy_selection(app: &mut PetriApp) {
  let Selection::Nodes(selected) = &app.selection else {
    return;
  };
  if selected.is_empty() {
    return;
  }
  let selected = selected.clone();

  let mut nodes = Vec::new();
  for &id in &selected {
    let Some(&pos) = app.positions.get(&id) else {
      continue;
    };
    let data = match id {
      NodeId::Place(p) if app.net.place_ids().any(|x| x == p) => ClipboardNode::Place {
        label: app.net.place_label(p).to_string(),
        tokens: app.net.tokens(p),
        color: app.colors.get(&p).copied(),
      },
      NodeId::Transition(t) if app.net.transition_ids().any(|x| x == t) => {
        ClipboardNode::Transition {
          label: app.net.transition_label(t).to_string(),
          rotation: app.rotation.get(&t).copied(),
        }
      }
      _ => continue,
    };
    nodes.push((id, data, pos));
  }
  if nodes.is_empty() {
    return;
  }

  let mut arcs_in = Vec::new();
  let mut arcs_out = Vec::new();
  for &id in &selected {
    if let NodeId::Transition(t) = id {
      for &(p, kind) in app.net.inputs(t) {
        if selected.contains(&NodeId::Place(p)) {
          arcs_in.push((NodeId::Place(p), NodeId::Transition(t), kind));
        }
      }
      for &(p, weight) in app.net.outputs(t) {
        if selected.contains(&NodeId::Place(p)) {
          arcs_out.push((NodeId::Transition(t), NodeId::Place(p), weight));
        }
      }
    }
  }
  app.clipboard = Some(Clipboard {
    nodes,
    arcs_in,
    arcs_out,
  });
}

/// World-space offset applied to a pasted selection so it lands next to the original instead of
/// exactly on top of it.
const PASTE_OFFSET: egui::Vec2 = egui::vec2(GRID_SPACING, GRID_SPACING);

fn paste_clipboard(app: &mut PetriApp) {
  let Some(clip) = app.clipboard.clone() else {
    return;
  };
  checkpoint(app);

  let mut mapping: HashMap<NodeId, NodeId> = HashMap::new();
  let mut new_selection = HashSet::new();
  for (old_id, data, pos) in &clip.nodes {
    let new_id = match data {
      ClipboardNode::Place {
        label,
        tokens,
        color,
      } => {
        app.next_place_n += 1;
        let id = app.net.add_place(label.clone());
        app.net.set_tokens(id, *tokens);
        if let Some(c) = color {
          app.colors.insert(id, *c);
        }
        NodeId::Place(id)
      }
      ClipboardNode::Transition { label, rotation } => {
        app.next_transition_n += 1;
        let id = app.net.add_transition(label.clone());
        if let Some(r) = rotation {
          app.rotation.insert(id, *r);
        }
        NodeId::Transition(id)
      }
    };
    app.positions.insert(new_id, *pos + PASTE_OFFSET);
    mapping.insert(*old_id, new_id);
    new_selection.insert(new_id);
  }
  for (from, to, kind) in &clip.arcs_in {
    if let (Some(&NodeId::Place(np)), Some(&NodeId::Transition(nt))) =
      (mapping.get(from), mapping.get(to))
    {
      let _ = app.net.add_arc_place_to_transition(np, nt, *kind);
    }
  }
  for (from, to, weight) in &clip.arcs_out {
    if let (Some(&NodeId::Transition(nt)), Some(&NodeId::Place(np))) =
      (mapping.get(from), mapping.get(to))
    {
      let _ = app.net.add_arc_transition_to_place(nt, np, *weight);
    }
  }
  app.selection = Selection::Nodes(new_selection);
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Align {
  /// Levels the group along whichever axis it's already loosely lined up on: a wider-than-tall
  /// selection becomes a row (shared y), a taller-than-wide one becomes a column (shared,
  /// centered x) — same call a "straighten" tool in a drawing app would make.
  Auto,
  Left,
  Center,
  Right,
}

/// Aligns every node in `nodes` along `align`. No-op below two nodes — there's nothing to align
/// relative to.
fn align_selected(app: &mut PetriApp, nodes: &HashSet<NodeId>, align: Align) {
  let positions: Vec<(NodeId, egui::Pos2)> = nodes
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
  let auto_row = (max_x - min_x) >= (max_y - min_y);

  for (n, p) in positions {
    let new_pos = match align {
      Align::Left => egui::pos2(min_x, p.y),
      Align::Center => egui::pos2(avg_x, p.y),
      Align::Right => egui::pos2(max_x, p.y),
      Align::Auto if auto_row => egui::pos2(p.x, avg_y),
      Align::Auto => egui::pos2(avg_x, p.y),
    };
    app.positions.insert(n, new_pos);
  }
}

/// `pos` is in world space.
fn handle_click(app: &mut PetriApp, pos: egui::Pos2) {
  let hit = hit_test(app, pos);
  // Any click that isn't "on the note already open for editing" (see the `Select` arm below,
  // the only branch that can re-set this) ends that note's edit session.
  app.editing_note = None;
  match app.mode {
    EditMode::AddPlace => {
      if hit.is_none() {
        checkpoint(app);
        app.next_place_n += 1;
        let id = app.net.add_place(format!("p{}", app.next_place_n));
        app.positions.insert(NodeId::Place(id), pos);
      }
    }
    EditMode::AddTransition => {
      if hit.is_none() {
        checkpoint(app);
        app.next_transition_n += 1;
        let id = app
          .net
          .add_transition(format!("t{}", app.next_transition_n));
        app.positions.insert(NodeId::Transition(id), pos);
      }
    }
    EditMode::AddNote => {
      if note_hit_test(app, pos).is_none() {
        checkpoint(app);
        let size = egui::vec2(180.0, 100.0);
        let id = app.notes.insert(crate::app::NoteData {
          pos: pos - size / 2.0,
          size,
          text: String::new(),
          color: None,
        });
        app.selection = Selection::Note(id);
        app.editing_note = Some(id);
      }
    }
    EditMode::Connect => match hit {
      Some(node) => match app.connect_from.take() {
        None => app.connect_from = Some(node),
        Some(from) => connect(app, from, node),
      },
      None => app.connect_from = None,
    },
    EditMode::Select => {
      if let Some(note_id) = note_hit_test(app, pos) {
        app.selection = Selection::Note(note_id);
        if app.reselecting_note {
          app.editing_note = Some(note_id);
        }
        return;
      }
      match hit {
        Some(node) => {
          app.selection = Selection::Nodes(HashSet::from([node]));
          if let NodeId::Transition(t) = node {
            fire_step(app, t);
          }
        }
        None => {
          app.selection = hit_test_arc(app, pos).unwrap_or(Selection::None);
        }
      }
    }
  }
}

fn apply_mode(app: &mut PetriApp, mode: EditMode) {
  app.mode = mode;
  app.connect_from = None;
}

pub fn canvas(app: &mut PetriApp, ui: &mut egui::Ui) {
  let visuals = ui.visuals().clone();
  let (response, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
  let rect = response.rect;
  app.canvas_rect = rect;
  let pan = app.pan;
  let zoom = app.zoom;
  let fallback = to_world(rect.center(), pan, zoom);

  // CentralPanel has no fill of its own here, so paint the canvas background explicitly.
  painter.rect_filled(rect, 0.0, theme::ink());
  if app.show_grid {
    draw_grid(&painter, rect, pan, zoom);
  }
  draw_net(app, &painter, fallback, pan, zoom, &visuals);
  draw_notes(app, &painter, pan, zoom, &visuals);

  // Route replay: `RouteModal::show` overwrites the live marking every frame so `draw_net`
  // above shows tokens moving along the recorded path (and dims everything off it). Editing,
  // selecting and note-editing are suspended while that's driving the canvas — pan/zoom below
  // stay on, so you can still look around.
  let editable = app.route_modal.is_none();

  if editable {
    note_edit_overlay(app, ui, pan, zoom);

    if app.mode == EditMode::Connect {
      if let Some(from) = app.connect_from {
        if let Some(mouse) = response.hover_pos() {
          draw_connect_preview(
            app,
            &painter,
            from,
            to_world(mouse, pan, zoom),
            fallback,
            pan,
            zoom,
            &visuals,
          );
        }
      }
    }

    if let (Some(start), Some(current)) = (app.marquee_start, app.marquee_current) {
      for node in nodes_in_rect(app, egui::Rect::from_two_pos(start, current)) {
        draw_selection_halo(
          app,
          &painter,
          node,
          to_screen(node_pos(app, node, fallback), pan, zoom),
          zoom,
          visuals.selection.bg_fill,
        );
      }
      draw_marquee(
        &painter,
        to_screen(start, pan, zoom),
        to_screen(current, pan, zoom),
        visuals.selection.bg_fill,
      );
    }
  }

  // Holding Space turns the primary button into a temporary pan grab. Node dragging,
  // marquee-select and click-to-place are all suspended while it's down.
  let space_held = ui.input(|i| i.key_down(egui::Key::Space));
  if response.hovered() {
    ui.ctx().set_cursor_icon(if space_held {
      if ui.input(|i| i.pointer.primary_down()) {
        egui::CursorIcon::Grabbing
      } else {
        egui::CursorIcon::Grab
      }
    } else {
      egui::CursorIcon::Default
    });
  }

  // Pan: middle-mouse drag, Space+primary drag, and trackpad/wheel scroll. Ctrl+scroll zooms
  // toward the cursor instead. All gated on hovering the canvas so they don't fight other
  // panels' scrolling.
  //
  // `i.smooth_scroll_delta` is NOT what fires during a ctrl/cmd+scroll gesture: egui's own
  // input handling (see `InputState::begin_pass`) detects the zoom modifier itself and routes
  // that scroll into `zoom_delta()` instead, leaving `smooth_scroll_delta` at zero. So the fix
  // isn't "check modifiers ourselves" (that races against something already zeroed) — it's to
  // read `zoom_delta()`, which is the same channel pinch-zoom gestures use.
  if response.hovered() {
    ui.input(|i| {
      if i.pointer.button_down(egui::PointerButton::Middle)
        || (space_held && i.pointer.button_down(egui::PointerButton::Primary))
      {
        app.pan += i.pointer.delta();
      }

      let zoom_factor = i.zoom_delta();
      if zoom_factor != 1.0 {
        if let Some(mouse) = i.pointer.hover_pos() {
          let new_zoom = (app.zoom * zoom_factor).clamp(ZOOM_MIN, ZOOM_MAX);
          let world_at_cursor = to_world(mouse, app.pan, app.zoom);
          app.pan = mouse.to_vec2() - world_at_cursor.to_vec2() * new_zoom;
          app.zoom = new_zoom;
        }
      }

      app.pan += i.smooth_scroll_delta;
    });
  }

  if !space_held && editable {
    if response.drag_started() {
      if let Some(pos) = response.interact_pointer_pos() {
        let world = to_world(pos, pan, zoom);
        if let Some(note_id) = note_resize_hit_test(app, world) {
          checkpoint(app);
          app.resizing_note = Some(note_id);
          app.selection = Selection::Note(note_id);
        } else if let Some(note_id) = note_hit_test(app, world) {
          checkpoint(app);
          app.dragging_note = Some(note_id);
          app.reselecting_note = app.selection == Selection::Note(note_id);
          app.selection = Selection::Note(note_id);
        } else {
          app.dragging = hit_test(app, world);
          if app.dragging.is_some() {
            checkpoint(app);
          } else if app.mode == EditMode::Select {
            app.marquee_start = Some(world);
            app.marquee_current = Some(world);
          }
        }
      }
    }
    // Held while dragging a node/note, snaps its position to the grid (same step `draw_grid`
    // dots at) instead of moving freely.
    let snap = ui.input(|i| i.modifiers.shift);
    if response.dragged() {
      if let Some(note_id) = app.resizing_note {
        let delta = response.drag_delta() / zoom;
        if let Some(note) = app.notes.get_mut(note_id) {
          note.size = (note.size + delta).max(NOTE_MIN_SIZE);
        }
      } else if let Some(note_id) = app.dragging_note {
        let raw_delta = response.drag_delta() / zoom;
        if let Some(note) = app.notes.get_mut(note_id) {
          note.pos = if snap {
            snap_to_grid(note.pos + raw_delta)
          } else {
            note.pos + raw_delta
          };
        }
      } else if let Some(node) = app.dragging {
        let raw_delta = response.drag_delta() / zoom;
        let delta = if snap {
          app
            .positions
            .get(&node)
            .map_or(raw_delta, |&pos| snap_to_grid(pos + raw_delta) - pos)
        } else {
          raw_delta
        };
        // Dragging any node that's part of an active multi-selection moves the whole group;
        // dragging an unselected (or singly-selected) node only ever moves that one node.
        let group = match &app.selection {
          Selection::Nodes(set) if set.len() > 1 && set.contains(&node) => Some(set.clone()),
          _ => None,
        };
        match group {
          Some(set) => {
            for n in set {
              if let Some(p) = app.positions.get_mut(&n) {
                *p += delta;
              }
            }
          }
          None => {
            if let Some(p) = app.positions.get_mut(&node) {
              *p += delta;
            }
          }
        }
      } else if app.marquee_start.is_some() {
        if let Some(pos) = response.interact_pointer_pos() {
          app.marquee_current = Some(to_world(pos, pan, zoom));
        }
      }
    }
    if response.drag_stopped() {
      app.dragging = None;
      app.dragging_note = None;
      app.resizing_note = None;
      if let (Some(start), Some(current)) = (app.marquee_start.take(), app.marquee_current.take()) {
        let matched = nodes_in_rect(app, egui::Rect::from_two_pos(start, current));
        app.selection = if matched.is_empty() {
          Selection::None
        } else {
          Selection::Nodes(matched)
        };
      }
    }

    if response.clicked() {
      if let Some(pos) = response.interact_pointer_pos() {
        handle_click(app, to_world(pos, pan, zoom));
      }
    }
  }

  // Right-click: figure out what was under the pointer once, on the click itself. The menu
  // stays open while the pointer wanders off the canvas, so recomputing the hit-test every
  // frame would lose the target the moment the mouse left.
  if editable {
    if response.secondary_clicked() {
      if let Some(pos) = response.interact_pointer_pos() {
        let world = to_world(pos, pan, zoom);
        let target = hit_test(app, world)
          .map(ContextTarget::Node)
          .unwrap_or_else(|| match hit_test_arc(app, world) {
            Some(Selection::ArcIn(p, t)) => ContextTarget::ArcIn(p, t),
            Some(Selection::ArcOut(t, p)) => ContextTarget::ArcOut(t, p),
            _ => ContextTarget::Empty(world),
          });
        // Right-clicking an item also selects it, so the inspector panel on the right stays in
        // sync with whatever the context menu is about to act on.
        app.selection = match target {
          ContextTarget::Node(node) => Selection::Nodes(HashSet::from([node])),
          ContextTarget::ArcIn(p, t) => Selection::ArcIn(p, t),
          ContextTarget::ArcOut(t, p) => Selection::ArcOut(t, p),
          ContextTarget::Empty(_) => Selection::None,
        };
        app.context_target = Some(target);
      }
    }
    response.context_menu(|ui| context_menu_contents(app, ui));

    let no_focus = ui.memory(|m| m.focused().is_none());
    if no_focus {
      let delete_pressed =
        ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
      if delete_pressed && !matches!(app.selection, Selection::None) {
        delete_selected(app);
      }

      ui.input(|i| {
        // `command` is Ctrl on Windows/Linux, Cmd on Mac — checked first on the letters it
        // shares with a mode shortcut (C, V) so e.g. Ctrl+V pastes instead of also flipping to
        // Connect/Select mode.
        if i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z) {
          redo(app);
        } else if i.modifiers.command && i.key_pressed(egui::Key::Z) {
          undo(app);
        }
        if i.modifiers.command && i.key_pressed(egui::Key::C) {
          copy_selection(app);
        } else if i.key_pressed(egui::Key::C) {
          apply_mode(app, EditMode::Connect);
        }
        if i.modifiers.command && i.key_pressed(egui::Key::V) {
          paste_clipboard(app);
        } else if i.key_pressed(egui::Key::V) {
          apply_mode(app, EditMode::Select);
        }
        if i.key_pressed(egui::Key::P) {
          apply_mode(app, EditMode::AddPlace);
        }
        if i.key_pressed(egui::Key::T) {
          apply_mode(app, EditMode::AddTransition);
        }
        if i.key_pressed(egui::Key::N) {
          apply_mode(app, EditMode::AddNote);
        }
      });
    }
  }
}

fn reset_view(app: &mut PetriApp) {
  app.pan = egui::Vec2::ZERO;
  app.zoom = 1.0;
}

/// Pans (without changing zoom) so `node`'s world position lands at the center of the canvas.
pub fn center_on_node(app: &mut PetriApp, node: NodeId) {
  let pos = node_pos(app, node, egui::Pos2::ZERO);
  app.pan = app.canvas_rect.center().to_vec2() - pos.to_vec2() * app.zoom;
}

/// A "Rotación [ 45°]" row: free-form drag/type field plus a `+45°` shortcut button.
fn rotation_control(ui: &mut egui::Ui, app: &mut PetriApp, t: crate::model::TransitionId) {
  ui.horizontal(|ui| {
    ui.label("Rotación");
    let mut angle = transition_angle(app, t);
    if ui
      .add(
        egui::DragValue::new(&mut angle)
          .range(0.0..=359.0)
          .suffix("°")
          .speed(1.0),
      )
      .changed()
    {
      set_rotation(app, t, angle);
    }
    if ui
      .add(
        egui::Button::new(icons::icon("rotate-cw", 12.0))
          .corner_radius(4.0)
          .min_size(egui::vec2(28.0, 24.0)),
      )
      .on_hover_text("+45°")
      .clicked()
    {
      nudge_rotation(app, t, 45.0);
    }
  });
}

/// Sets up a flat, cohesive look for a dropdown/context menu's contents: roomier row padding,
/// and a muted hover fill. `menu_item`/`danger_menu_item`/raw `Button`s in these scopes opt out
/// of the *idle* frame themselves via `.frame_when_inactive(false)`, so only hover/press ever
/// paint a background — dialed down here so it reads as a highlight, not a loud box.
fn begin_flat_menu(ui: &mut egui::Ui) {
  ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
  let muted = theme::surface_hover().gamma_multiply(0.5);
  ui.visuals_mut().widgets.hovered.weak_bg_fill = muted;
  ui.visuals_mut().widgets.hovered.bg_fill = muted;
}

fn menu_item(ui: &mut egui::Ui, icon_name: &'static str, label: &str) -> egui::Response {
  ui.add(
    egui::Button::new((icons::icon(icon_name, 13.0), label))
      .corner_radius(6.0)
      .frame_when_inactive(false),
  )
}

/// A menu row that toggles something on/off — a button, not a checkbox: same icon+label shape
/// as `menu_item`, but tinted with the app's selected/active style when `checked` (matching how
/// the toolbar's mode buttons show which one is active) and a check glyph up front.
fn toggle_menu_item(ui: &mut egui::Ui, checked: bool, label: &str) -> egui::Response {
  let check = icons::icon("check", 13.0).color(if checked {
    ui.visuals().text_color()
  } else {
    egui::Color32::TRANSPARENT
  });
  let mut button = egui::Button::new((check, label))
    .corner_radius(6.0)
    // `.selected()` pulls in the theme's full-strength accent fill, which reads as a glaring
    // pill next to the rest of this flat menu — a faint tint is enough to say "this is on".
    .frame_when_inactive(checked);
  if checked {
    button = button
      .fill(theme::accent().gamma_multiply(0.18))
      .frame(true);
  }
  ui.add(button)
}

fn danger_menu_item(ui: &mut egui::Ui, icon_name: &'static str, label: &str) -> egui::Response {
  ui.add(
    egui::Button::new((
      icons::icon(icon_name, 13.0).color(theme::danger()),
      egui::RichText::new(label).color(theme::danger()),
    ))
    .corner_radius(6.0)
    .frame_when_inactive(false),
  )
}

/// Right-click menu for whatever `app.context_target` captured when the menu opened (see the
/// `secondary_clicked` handling in `canvas`). `app.selection` was set to match the same target,
/// so the delete/edit actions below just reuse `delete_selected`.
fn context_menu_contents(app: &mut PetriApp, ui: &mut egui::Ui) {
  let Some(target) = app.context_target else {
    ui.close();
    return;
  };
  ui.set_min_width(190.0);
  begin_flat_menu(ui);
  match target {
    ContextTarget::Node(NodeId::Place(p)) if app.net.place_ids().any(|id| id == p) => {
      ui.add(
        egui::TextEdit::singleline(app.net.place_label_mut(p).unwrap())
          .font(egui::TextStyle::Body)
          .frame(egui::Frame::NONE)
          .desired_width(160.0),
      );
      ui.separator();
      if menu_item(ui, "plus", "Agregar token").clicked() {
        let tokens = app.net.tokens(p);
        app.net.set_tokens(p, tokens + 1);
        ui.close();
      }
      let has_tokens = app.net.tokens(p) > 0;
      if ui
        .add_enabled(
          has_tokens,
          egui::Button::new((icons::icon("minus", 13.0), "Quitar token"))
            .corner_radius(6.0)
            .frame_when_inactive(false),
        )
        .clicked()
      {
        let tokens = app.net.tokens(p);
        app.net.set_tokens(p, tokens.saturating_sub(1));
        ui.close();
      }
      if ui
        .add_enabled(
          has_tokens,
          egui::Button::new((icons::icon("x", 13.0), "Vaciar tokens"))
            .corner_radius(6.0)
            .frame_when_inactive(false),
        )
        .clicked()
      {
        app.net.set_tokens(p, 0);
        ui.close();
      }
      ui.separator();
      if danger_menu_item(ui, "trash-2", "Eliminar").clicked() {
        delete_selected(app);
        ui.close();
      }
    }
    ContextTarget::Node(NodeId::Transition(t)) if app.net.transition_ids().any(|id| id == t) => {
      ui.add(
        egui::TextEdit::singleline(app.net.transition_label_mut(t).unwrap())
          .font(egui::TextStyle::Body)
          .frame(egui::Frame::NONE)
          .desired_width(160.0),
      );
      ui.separator();
      let marking = app.net.marking();
      let enabled = fire::enabled_transitions(&app.net, &marking).contains(&t);
      if ui
        .add_enabled(
          enabled,
          egui::Button::new((icons::icon("zap", 13.0), "Disparar"))
            .corner_radius(6.0)
            .frame_when_inactive(false),
        )
        .clicked()
      {
        fire_step(app, t);
        ui.close();
      }
      rotation_control(ui, app, t);
      ui.separator();
      if danger_menu_item(ui, "trash-2", "Eliminar").clicked() {
        delete_selected(app);
        ui.close();
      }
    }
    ContextTarget::ArcIn(p, t) => {
      arrow_row(ui, app.net.place_label(p), app.net.transition_label(t));
      ui.weak("Arco de entrada");
      ui.separator();
      if let Some(&(_, current)) = app.net.inputs(t).iter().find(|(place, _)| *place == p) {
        let current_tag = ArcKindTag::from_kind(current);
        let weight = current.weight();
        ui.horizontal(|ui| {
          for candidate in [ArcKindTag::Consume, ArcKindTag::Peek, ArcKindTag::Inhibit] {
            let btn = egui::Button::new(icons::icon(candidate.icon(), 14.0))
              .selected(current_tag == candidate)
              .corner_radius(6.0)
              .min_size(egui::vec2(36.0, 30.0));
            if ui.add(btn).on_hover_text(candidate.tooltip()).clicked() {
              let w = if candidate == ArcKindTag::Consume {
                weight
              } else {
                weight.min(1)
              };
              app.net.remove_arc_place_to_transition(p, t);
              let _ = app
                .net
                .add_arc_place_to_transition(p, t, candidate.to_kind(w));
              ui.close();
            }
          }
        });
      }
      ui.separator();
      if danger_menu_item(ui, "trash-2", "Eliminar arco").clicked() {
        delete_selected(app);
        ui.close();
      }
    }
    ContextTarget::ArcOut(t, p) => {
      arrow_row(ui, app.net.transition_label(t), app.net.place_label(p));
      ui.weak("Arco de salida");
      ui.separator();
      if let Some(&(_, w)) = app.net.outputs(t).iter().find(|(place, _)| *place == p) {
        if menu_item(ui, "plus", "Aumentar peso").clicked() {
          app.net.remove_arc_transition_to_place(t, p);
          let _ = app.net.add_arc_transition_to_place(t, p, (w + 1).min(99));
          ui.close();
        }
        if ui
          .add_enabled(
            w > 1,
            egui::Button::new((icons::icon("minus", 13.0), "Disminuir peso"))
              .corner_radius(6.0)
              .frame_when_inactive(false),
          )
          .clicked()
        {
          app.net.remove_arc_transition_to_place(t, p);
          let _ = app.net.add_arc_transition_to_place(t, p, w - 1);
          ui.close();
        }
      }
      ui.separator();
      if danger_menu_item(ui, "trash-2", "Eliminar arco").clicked() {
        delete_selected(app);
        ui.close();
      }
    }
    ContextTarget::Empty(world) => {
      if menu_item(ui, "circle", "Agregar place aquí").clicked() {
        app.next_place_n += 1;
        let id = app.net.add_place(format!("p{}", app.next_place_n));
        app.positions.insert(NodeId::Place(id), world);
        ui.close();
      }
      if menu_item(ui, "rectangle-vertical", "Agregar transition aquí").clicked() {
        app.next_transition_n += 1;
        let id = app
          .net
          .add_transition(format!("t{}", app.next_transition_n));
        app.positions.insert(NodeId::Transition(id), world);
        ui.close();
      }
      ui.separator();
      let has_any =
        app.net.place_ids().next().is_some() || app.net.transition_ids().next().is_some();
      if ui
        .add_enabled(
          has_any,
          egui::Button::new((icons::icon("box-select", 13.0), "Seleccionar todo"))
            .corner_radius(6.0)
            .frame_when_inactive(false),
        )
        .clicked()
      {
        let all: HashSet<NodeId> = app
          .net
          .place_ids()
          .map(NodeId::Place)
          .chain(app.net.transition_ids().map(NodeId::Transition))
          .collect();
        app.selection = Selection::Nodes(all);
        ui.close();
      }
    }
    _ => {
      ui.weak("Elemento eliminado");
      ui.close();
    }
  }
}

const FILE_EXTENSION: &str = "gpn";
const MAX_RECENT: usize = 8;

/// Moves `path` to the front of `app.recent`, deduplicating and capping its length. Persisted
/// to disk by eframe's own storage (`PetriApp::save`), on its regular save cycle and on exit.
fn remember_recent(app: &mut PetriApp, path: std::path::PathBuf) {
  app.recent.retain(|p| p != &path);
  app.recent.insert(0, path);
  app.recent.truncate(MAX_RECENT);
}

/// Resets `app` to a blank, never-saved project (positions, view, undo history — everything
/// except the recent-files list).
fn file_new(app: &mut PetriApp) {
  let recent = std::mem::take(&mut app.recent);
  *app = PetriApp::new();
  app.recent = recent;
}

fn file_display_name(path: &std::path::Path) -> std::borrow::Cow<'_, str> {
  path
    .file_name()
    .map(|n| n.to_string_lossy())
    .unwrap_or_else(|| path.to_string_lossy())
}

fn open_path(app: &mut PetriApp, path: std::path::PathBuf) {
  match crate::project::load(&path) {
    Ok(loaded) => {
      let recent = std::mem::take(&mut app.recent);
      *app = PetriApp::new();
      app.recent = recent;
      app.net = loaded.net;
      app.positions = loaded.positions;
      app.rotation = loaded.rotation;
      app.colors = loaded.colors;
      app.notes = loaded.notes;
      app.next_place_n = loaded.next_place_n;
      app.next_transition_n = loaded.next_transition_n;
      log::info!("opened project: {}", path.display());
      app.notify(
        egui_toast::ToastKind::Success,
        format!("Abierto: {}", file_display_name(&path)),
      );
      remember_recent(app, path.clone());
      app.file_path = Some(path);
    }
    Err(e) => {
      log::error!("failed to open project {}: {e}", path.display());
      app.notify(
        egui_toast::ToastKind::Error,
        format!("No se pudo abrir el proyecto: {e}"),
      );
    }
  }
}

fn file_open(app: &mut PetriApp) {
  let Some(path) = rfd::FileDialog::new()
    .add_filter("gpn", &[FILE_EXTENSION])
    .pick_file()
  else {
    return;
  };
  open_path(app, path);
}

fn file_save_as(app: &mut PetriApp) {
  let Some(path) = rfd::FileDialog::new()
    .add_filter("gpn", &[FILE_EXTENSION])
    .set_file_name(format!("net.{FILE_EXTENSION}"))
    .save_file()
  else {
    return;
  };
  match crate::project::save(app, &path) {
    Ok(()) => {
      log::info!("saved project: {}", path.display());
      app.notify(
        egui_toast::ToastKind::Success,
        format!("Guardado: {}", file_display_name(&path)),
      );
      remember_recent(app, path.clone());
      app.file_path = Some(path);
    }
    Err(e) => {
      log::error!("failed to save project {}: {e}", path.display());
      app.notify(
        egui_toast::ToastKind::Error,
        format!("No se pudo guardar el proyecto: {e}"),
      );
    }
  }
}

fn file_save(app: &mut PetriApp) {
  match app.file_path.clone() {
    Some(path) => match crate::project::save(app, &path) {
      Ok(()) => {
        log::info!("saved project: {}", path.display());
        app.notify(
          egui_toast::ToastKind::Success,
          format!("Guardado: {}", file_display_name(&path)),
        );
        remember_recent(app, path);
      }
      Err(e) => {
        log::error!("failed to save project {}: {e}", path.display());
        app.notify(
          egui_toast::ToastKind::Error,
          format!("No se pudo guardar el proyecto: {e}"),
        );
      }
    },
    None => file_save_as(app),
  }
}

/// Top menu bar: identity mark on the left, then File/Edit/View menus.
pub fn menu_bar(app: &mut PetriApp, ui: &mut egui::Ui, ctx: &egui::Context) {
  let no_focus = ctx.memory(|m| m.focused().is_none());
  if no_focus && ctx.input(|i| i.key_pressed(egui::Key::F1)) {
    crate::dock::toggle_help(app);
  }

  egui::MenuBar::new().ui(ui, |ui| {
    // `MenuBar` forces a cramped `(2, 0)` button padding on its direct contents (fine for a
    // dense app menu bar in general, but reads as squished here) — give the top-level
    // Archivo/Editar/Ver openers and the theme toggle some breathing room instead.
    ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
    ui.horizontal(|ui| {
      ui.label(icons::icon("workflow", 16.0).color(theme::accent()));
      ui.add_space(2.0);
      ui.label(egui::RichText::new("petricrab").strong().size(14.0));
      ui.weak(concat!("v", env!("CARGO_PKG_VERSION")));
    });
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(4.0);

    ui.menu_button("Archivo", |ui| {
      ui.set_min_width(170.0);
      begin_flat_menu(ui);
      if menu_item(ui, "file-plus", "Nuevo").clicked() {
        file_new(app);
        ui.close();
      }
      if menu_item(ui, "folder-open", "Abrir…").clicked() {
        file_open(app);
        ui.close();
      }
      if menu_item(ui, "save", "Guardar").clicked() {
        file_save(app);
        ui.close();
      }
      if menu_item(ui, "save", "Guardar como…").clicked() {
        file_save_as(app);
        ui.close();
      }
      ui.separator();
      ui.add_enabled_ui(!app.recent.is_empty(), |ui| {
        ui.menu_button("Recientes", |ui| {
          ui.set_min_width(220.0);
          if app.recent.is_empty() {
            ui.weak("(ninguno)");
          } else {
            for path in app.recent.clone() {
              let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
              if ui
                .add(egui::Button::new(name).frame(false))
                .on_hover_text(path.to_string_lossy())
                .clicked()
              {
                open_path(app, path);
                ui.close();
              }
            }
          }
        });
      });
      ui.separator();
      if menu_item(ui, "log-out", "Salir").clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
      }
    });

    ui.menu_button("Editar", |ui| {
      ui.set_min_width(170.0);
      begin_flat_menu(ui);
      if ui
        .add_enabled(
          !app.undo_stack.is_empty(),
          egui::Button::new((icons::icon("undo-2", 13.0), "Deshacer"))
            .corner_radius(6.0)
            .frame_when_inactive(false),
        )
        .on_hover_text("Ctrl+Z")
        .clicked()
      {
        undo(app);
        ui.close();
      }
      if ui
        .add_enabled(
          !app.redo_stack.is_empty(),
          egui::Button::new((icons::icon("redo-2", 13.0), "Rehacer"))
            .corner_radius(6.0)
            .frame_when_inactive(false),
        )
        .on_hover_text("Ctrl+Shift+Z")
        .clicked()
      {
        redo(app);
        ui.close();
      }
      ui.separator();
      if ui
        .add_enabled(
          matches!(&app.selection, Selection::Nodes(n) if !n.is_empty()),
          egui::Button::new((icons::icon("copy", 13.0), "Copiar"))
            .corner_radius(6.0)
            .frame_when_inactive(false),
        )
        .on_hover_text("Ctrl+C")
        .clicked()
      {
        copy_selection(app);
        ui.close();
      }
      if ui
        .add_enabled(
          app.clipboard.is_some(),
          egui::Button::new((icons::icon("clipboard-paste", 13.0), "Pegar"))
            .corner_radius(6.0)
            .frame_when_inactive(false),
        )
        .on_hover_text("Ctrl+V")
        .clicked()
      {
        paste_clipboard(app);
        ui.close();
      }
    });

    // Rect captured for the tutorial's last step to spotlight this button specifically instead
    // of the whole menu bar — set every frame, read only while a tutorial is open. "Propiedades
    // del net" lives under "Ver", not "Editar".
    app.menu_ver_rect = ui
      .menu_button("Ver", |ui| {
        ui.set_min_width(190.0);
        begin_flat_menu(ui);
        if toggle_menu_item(ui, app.show_grid, "Mostrar grilla").clicked() {
          app.show_grid = !app.show_grid;
        }
        if menu_item(ui, "locate-fixed", "Reiniciar vista").clicked() {
          reset_view(app);
          ui.close();
        }
        ui.separator();
        if toggle_menu_item(
          ui,
          app.reachability.is_some(),
          "Explorar espacio de estados",
        )
        .clicked()
        {
          crate::dock::toggle_reachability(app);
        }
        if toggle_menu_item(ui, app.properties.is_some(), "Propiedades del net").clicked() {
          crate::dock::toggle_properties(app);
        }
        let show_outline = app.dock.find_tab(&crate::dock::DockTab::Outline).is_some();
        if toggle_menu_item(ui, show_outline, "Estructura").clicked() {
          crate::dock::toggle_outline(app);
        }
      })
      .response
      .rect;

    ui.menu_button("Ayuda", |ui| {
      ui.set_min_width(190.0);
      begin_flat_menu(ui);
      let show_help = app.dock.find_tab(&crate::dock::DockTab::Help).is_some();
      if toggle_menu_item(ui, show_help, "Ayuda (F1)").clicked() {
        crate::dock::toggle_help(app);
      }
      ui.separator();
      if menu_item(ui, "graduation-cap", "Ver tutorial").clicked() {
        app.tutorial = Some(crate::tutorial::TutorialState::new());
        ui.close();
      }
    });

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
      let icon_name = if app.light_mode { "moon" } else { "sun" };
      let tooltip = if app.light_mode {
        "Cambiar a modo oscuro"
      } else {
        "Cambiar a modo claro"
      };
      if ui
        .add(egui::Button::new(icons::icon(icon_name, 15.0)).corner_radius(6.0))
        .on_hover_text(tooltip)
        .clicked()
      {
        app.light_mode = !app.light_mode;
        theme::set_light(ctx, app.light_mode);
      }
    });
  });
}

pub(crate) fn section_label(ui: &mut egui::Ui, text: &str) {
  ui.label(
    egui::RichText::new(text.to_uppercase())
      .size(11.0)
      .color(ui.visuals().weak_text_color())
      .strong(),
  );
}

/// A grouped, rounded card matching the toolbar/simulate-popup look, used to visually separate
/// inspector sections from the flat panel background instead of leaving controls floating loose.
pub(crate) fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
  egui::Frame::default()
    .fill(ui.visuals().faint_bg_color)
    .corner_radius(10.0)
    .inner_margin(egui::Margin::symmetric(12, 10))
    .show(ui, add_contents);
}

fn destructive_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
  let danger = theme::danger();
  ui.add(
    egui::Button::new((
      icons::icon("trash-2", 14.0).color(danger),
      egui::RichText::new(label).color(danger),
    ))
    .corner_radius(6.0)
    .stroke(egui::Stroke::new(1.0, danger.gamma_multiply(0.4))),
  )
  .on_hover_text("Supr")
}

/// Inspector header: a round icon badge (not a boxed card, just an avatar) next to the
/// entity's editable name and its kind, e.g. a place's filled-circle glyph beside "p1" / "Place".
fn entity_title(ui: &mut egui::Ui, icon_name: &'static str, name: &mut String, kind: &str) {
  ui.horizontal(|ui| {
    egui::Frame::default()
      .fill(ui.visuals().faint_bg_color)
      .corner_radius(14.0)
      .inner_margin(egui::Margin::same(7))
      .show(ui, |ui| {
        ui.label(icons::icon(icon_name, 15.0));
      });
    ui.add_space(4.0);
    ui.vertical(|ui| {
      ui.add(
        egui::TextEdit::singleline(name)
          .font(egui::FontId::proportional(15.0))
          .frame(egui::Frame::NONE)
          .desired_width(140.0),
      );
      ui.weak(kind);
    });
  });
}

/// `a (icon) b` row, used instead of a raw unicode arrow character for arc summaries.
fn arrow_row(ui: &mut egui::Ui, a: &str, b: &str) {
  ui.horizontal(|ui| {
    ui.label(egui::RichText::new(a).strong());
    ui.label(icons::icon("arrow-right", 12.0));
    ui.label(egui::RichText::new(b).strong());
  });
}

/// A labeled +/- stepper instead of a bare debug-looking number field. Returns true on change.
fn stepper(ui: &mut egui::Ui, label: &str, value: &mut u32, min: u32, max: u32) -> bool {
  let mut changed = false;
  ui.horizontal(|ui| {
    ui.label(label);
    let step_btn = |ui: &mut egui::Ui, icon_name: &'static str| {
      ui.add(
        egui::Button::new(icons::icon(icon_name, 12.0))
          .corner_radius(4.0)
          .min_size(egui::vec2(28.0, 24.0)),
      )
    };
    if step_btn(ui, "minus").clicked() && *value > min {
      *value -= 1;
      changed = true;
    }
    ui.label(egui::RichText::new(value.to_string()).monospace().strong());
    if step_btn(ui, "plus").clicked() && *value < max {
      *value += 1;
      changed = true;
    }
  });
  changed
}

#[derive(Clone, Copy, PartialEq)]
enum ArcKindTag {
  Consume,
  Peek,
  Inhibit,
}

impl ArcKindTag {
  fn from_kind(kind: ArcKind) -> Self {
    match kind {
      ArcKind::Consume(_) => Self::Consume,
      ArcKind::Peek(_) => Self::Peek,
      ArcKind::Inhibit(_) => Self::Inhibit,
    }
  }

  fn to_kind(self, weight: u32) -> ArcKind {
    match self {
      Self::Consume => ArcKind::Consume(weight),
      Self::Peek => ArcKind::Peek(weight),
      Self::Inhibit => ArcKind::Inhibit(weight),
    }
  }

  fn icon(self) -> &'static str {
    match self {
      Self::Consume => "arrow-right",
      Self::Peek => "eye",
      Self::Inhibit => "ban",
    }
  }

  fn tooltip(self) -> &'static str {
    match self {
      Self::Consume => "Consume: resta tokens al disparar",
      Self::Peek => "Peek: requiere tokens pero no los consume",
      Self::Inhibit => "Inhibit: bloquea mientras haya tokens",
    }
  }
}

fn arc_in_editor(
  app: &mut PetriApp,
  ui: &mut egui::Ui,
  p: crate::model::PlaceId,
  t: crate::model::TransitionId,
) {
  let Some(&(_, current)) = app.net.inputs(t).iter().find(|(place, _)| *place == p) else {
    return;
  };
  let mut tag = ArcKindTag::from_kind(current);
  let mut weight = current.weight();
  let mut changed = false;

  ui.horizontal(|ui| {
    for candidate in [ArcKindTag::Consume, ArcKindTag::Peek, ArcKindTag::Inhibit] {
      let btn = egui::Button::new(icons::icon(candidate.icon(), 15.0))
        .selected(tag == candidate)
        .corner_radius(6.0)
        .min_size(egui::vec2(36.0, 30.0));
      if ui.add(btn).on_hover_text(candidate.tooltip()).clicked() {
        tag = candidate;
        changed = true;
      }
    }
  });
  // ponytail: petricrab-core's analysis bridge only models Peek/Inhibit as Murata's classic
  // unweighted arcs ("some token" / "no token"); Consume is the only kind with a real weight
  // there. Capping the stepper at 1 for Peek/Inhibit keeps every arc the editor can create
  // faithfully representable by the analysis (see analysis.rs::to_analysis_net). Upgrade path:
  // weighted Peek/Inhibit in petricrab-core if this cap ever needs to go.
  let max_weight = if tag == ArcKindTag::Consume { 99 } else { 1 };
  if weight > max_weight {
    weight = max_weight;
    changed = true;
  }
  if stepper(ui, "Peso", &mut weight, 1, max_weight) {
    changed = true;
  }

  if changed {
    app.net.remove_arc_place_to_transition(p, t);
    let _ = app
      .net
      .add_arc_place_to_transition(p, t, tag.to_kind(weight));
  }
}

fn arc_out_editor(
  app: &mut PetriApp,
  ui: &mut egui::Ui,
  t: crate::model::TransitionId,
  p: crate::model::PlaceId,
) {
  let Some(&(_, current_weight)) = app.net.outputs(t).iter().find(|(place, _)| *place == p) else {
    return;
  };
  let mut weight = current_weight;
  if stepper(ui, "Peso", &mut weight, 1, 99) {
    app.net.remove_arc_transition_to_place(t, p);
    let _ = app.net.add_arc_transition_to_place(t, p, weight);
  }
}

pub fn selection_panel(app: &mut PetriApp, ui: &mut egui::Ui) {
  section_label(ui, "Selección");
  ui.add_space(8.0);

  // Cloned once so the rest of this function can freely call `&mut app.*` (e.g.
  // `delete_selected`, the arc editors) without fighting a live borrow of `app.selection`.
  let selection = app.selection.clone();
  match selection {
    Selection::Nodes(nodes) if nodes.len() == 1 => {
      let node = *nodes.iter().next().unwrap();
      match node {
        NodeId::Place(p) if app.net.place_ids().any(|id| id == p) => {
          entity_title(ui, "circle", app.net.place_label_mut(p).unwrap(), "Place");
          ui.add_space(12.0);
          ui.separator();
          ui.add_space(10.0);
          let mut tokens = app.net.tokens(p);
          if stepper(ui, "Tokens", &mut tokens, 0, 999) {
            app.net.set_tokens(p, tokens);
          }
          ui.add_space(10.0);
          ui.horizontal(|ui| {
            ui.label("Color");
            let mut color = app.colors.get(&p).copied().unwrap_or(theme::ink());
            if ui.color_edit_button_srgba(&mut color).changed() {
              app.colors.insert(p, color);
            }
            if app.colors.contains_key(&p)
              && ui
                .add(egui::Button::new(icons::icon("rotate-ccw", 13.0)).frame(false))
                .on_hover_text("Restablecer color")
                .clicked()
            {
              app.colors.remove(&p);
            }
          });
          ui.add_space(14.0);
          if destructive_button(ui, "Eliminar").clicked() {
            delete_selected(app);
          }
        }
        NodeId::Transition(t) if app.net.transition_ids().any(|id| id == t) => {
          entity_title(
            ui,
            "rectangle-vertical",
            app.net.transition_label_mut(t).unwrap(),
            "Transition",
          );
          ui.add_space(12.0);
          ui.separator();
          ui.add_space(10.0);
          rotation_control(ui, app, t);
          ui.add_space(14.0);
          if destructive_button(ui, "Eliminar").clicked() {
            delete_selected(app);
          }
        }
        _ => {
          app.selection = Selection::None;
          ui.weak("Nada seleccionado");
        }
      }
    }
    Selection::Nodes(nodes) if nodes.len() > 1 => {
      ui.label(egui::RichText::new(format!("{} elementos", nodes.len())).strong());
      ui.weak("Tocá un elemento para editarlo");
      ui.add_space(10.0);
      ui.separator();
      ui.add_space(10.0);

      section_label(ui, "Alinear");
      ui.add_space(6.0);
      ui.horizontal(|ui| {
        let align_btn = |ui: &mut egui::Ui, icon_name: &'static str, tooltip: &str| {
          ui
            .add(
              egui::Button::new(icons::icon(icon_name, 14.0))
                .corner_radius(6.0)
                .min_size(egui::vec2(36.0, 30.0)),
            )
            .on_hover_text(tooltip)
        };
        if align_btn(ui, "wand-sparkles", "Auto").clicked() {
          align_selected(app, &nodes, Align::Auto);
        }
        if align_btn(ui, "align-horizontal-justify-start", "Izquierda").clicked() {
          align_selected(app, &nodes, Align::Left);
        }
        if align_btn(ui, "align-horizontal-justify-center", "Centro").clicked() {
          align_selected(app, &nodes, Align::Center);
        }
        if align_btn(ui, "align-horizontal-justify-end", "Derecha").clicked() {
          align_selected(app, &nodes, Align::Right);
        }
      });
      ui.add_space(10.0);
      ui.separator();
      ui.add_space(10.0);

      if let Some(focus) = app.selection_focus {
        if !nodes.contains(&focus) {
          app.selection_focus = None;
        }
      }

      let mut sorted: Vec<NodeId> = nodes.iter().copied().collect();
      sorted.sort();
      for node in sorted {
        let (icon_name, exists): (&'static str, bool) = match node {
          NodeId::Place(p) => ("circle", app.net.place_ids().any(|id| id == p)),
          NodeId::Transition(t) => (
            "rectangle-vertical",
            app.net.transition_ids().any(|id| id == t),
          ),
        };
        // Deleted mid-selection: just skip it, the entry drops out on the next click.
        if !exists {
          continue;
        }
        card(ui, |ui| {
          let focused = app.selection_focus == Some(node);
          ui.horizontal(|ui| {
            let toggle = ui.add(
              egui::Button::new(icons::icon(icon_name, 13.0))
                .frame(false)
                .selected(focused),
            );
            if toggle.clicked() {
              app.selection_focus = if focused { None } else { Some(node) };
            }
            match node {
              NodeId::Place(p) => {
                ui.add(
                  egui::TextEdit::singleline(app.net.place_label_mut(p).unwrap())
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
                );
              }
              NodeId::Transition(t) => {
                ui.add(
                  egui::TextEdit::singleline(app.net.transition_label_mut(t).unwrap())
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
                );
              }
            }
          });
          if focused {
            ui.add_space(8.0);
            match node {
              NodeId::Place(p) => {
                let mut tokens = app.net.tokens(p);
                if stepper(ui, "Tokens", &mut tokens, 0, 999) {
                  app.net.set_tokens(p, tokens);
                }
              }
              NodeId::Transition(t) => {
                rotation_control(ui, app, t);
              }
            }
          }
        });
        ui.add_space(6.0);
      }

      ui.add_space(8.0);
      if destructive_button(ui, "Eliminar todos").clicked() {
        delete_selected(app);
        app.selection_focus = None;
      }
    }
    Selection::ArcIn(p, t) => {
      arrow_row(ui, app.net.place_label(p), app.net.transition_label(t));
      ui.weak("Arco de entrada");
      ui.add_space(10.0);
      ui.separator();
      ui.add_space(10.0);
      arc_in_editor(app, ui, p, t);
      ui.add_space(14.0);
      if destructive_button(ui, "Eliminar arco").clicked() {
        delete_selected(app);
      }
    }
    Selection::ArcOut(t, p) => {
      arrow_row(ui, app.net.transition_label(t), app.net.place_label(p));
      ui.weak("Arco de salida");
      ui.add_space(10.0);
      ui.separator();
      ui.add_space(10.0);
      arc_out_editor(app, ui, t, p);
      ui.add_space(14.0);
      if destructive_button(ui, "Eliminar arco").clicked() {
        delete_selected(app);
      }
    }
    Selection::Note(id) if app.notes.contains_key(id) => {
      ui.horizontal(|ui| {
        ui.label(icons::icon("sticky-note", 15.0));
        ui.label(egui::RichText::new("Nota").strong());
      });
      ui.add_space(12.0);
      ui.separator();
      ui.add_space(10.0);
      if let Some(note) = app.notes.get_mut(id) {
        ui.add(
          egui::TextEdit::multiline(&mut note.text)
            .desired_rows(6)
            .desired_width(f32::INFINITY)
            .hint_text("Escribí lo que quieras…"),
        );
      }
      ui.add_space(10.0);
      ui.horizontal(|ui| {
        ui.label("Color");
        let mut color = app
          .notes
          .get(id)
          .and_then(|n| n.color)
          .unwrap_or(theme::surface_raised());
        if ui.color_edit_button_srgba(&mut color).changed()
          && let Some(note) = app.notes.get_mut(id)
        {
          note.color = Some(color);
        }
        if app.notes.get(id).is_some_and(|n| n.color.is_some())
          && ui
            .add(egui::Button::new(icons::icon("rotate-ccw", 13.0)).frame(false))
            .on_hover_text("Restablecer color")
            .clicked()
          && let Some(note) = app.notes.get_mut(id)
        {
          note.color = None;
        }
      });
      ui.add_space(14.0);
      if destructive_button(ui, "Eliminar").clicked() {
        delete_selected(app);
      }
    }
    _ => {
      ui.weak("Nada seleccionado");
    }
  }
}

/// Full list of the net's places/transitions, click-to-select-and-center — the "where is
/// everything" view for nets too big to eyeball on the canvas at a glance.
pub fn outline_panel(app: &mut PetriApp, ui: &mut egui::Ui) {
  egui::ScrollArea::vertical().show(ui, |ui| {
    section_label(ui, "Places");
    ui.add_space(4.0);
    let place_ids: Vec<_> = app.net.place_ids().collect();
    if place_ids.is_empty() {
      ui.weak("(ninguno)");
    }
    for p in place_ids {
      let selected = app.selection == Selection::Nodes([NodeId::Place(p)].into());
      let row = ui.add(
        egui::Button::new(format!(
          "{}   ·   {} tok.",
          app.net.place_label(p),
          app.net.tokens(p)
        ))
        .frame(false)
        .selected(selected)
        .min_size(egui::vec2(ui.available_width(), 0.0)),
      );
      if row.clicked() {
        app.selection = Selection::Nodes([NodeId::Place(p)].into());
        center_on_node(app, NodeId::Place(p));
      }
    }
    ui.add_space(10.0);

    section_label(ui, "Transitions");
    ui.add_space(4.0);
    let marking = app.net.marking();
    let enabled = fire::enabled_transitions(&app.net, &marking);
    let transition_ids: Vec<_> = app.net.transition_ids().collect();
    if transition_ids.is_empty() {
      ui.weak("(ninguna)");
    }
    for t in transition_ids {
      let selected = app.selection == Selection::Nodes([NodeId::Transition(t)].into());
      let icon_name = if enabled.contains(&t) { "zap" } else { "ban" };
      let row = ui.add(
        egui::Button::new((icons::icon(icon_name, 13.0), app.net.transition_label(t)))
          .frame(false)
          .selected(selected)
          .min_size(egui::vec2(ui.available_width(), 0.0)),
      );
      if row.clicked() {
        app.selection = Selection::Nodes([NodeId::Transition(t)].into());
        center_on_node(app, NodeId::Transition(t));
      }
    }
  });
}

fn mode_icon_and_tooltip(mode: EditMode) -> (&'static str, &'static str) {
  match mode {
    EditMode::Select => ("mouse-pointer-2", "Seleccionar / token game (V)"),
    EditMode::AddPlace => ("circle", "Agregar place (P)"),
    EditMode::AddTransition => ("rectangle-vertical", "Agregar transition (T)"),
    EditMode::Connect => ("cable", "Conectar arco (C)"),
    EditMode::AddNote => ("sticky-note", "Agregar nota (N)"),
  }
}

pub fn toolbar(app: &mut PetriApp, ui: &mut egui::Ui) {
  ui.horizontal(|ui| {
    for mode in [
      EditMode::Select,
      EditMode::AddPlace,
      EditMode::AddTransition,
      EditMode::Connect,
      EditMode::AddNote,
    ] {
      let (icon_name, tooltip) = mode_icon_and_tooltip(mode);
      let button = egui::Button::new(icons::icon(icon_name, 17.0))
        .selected(app.mode == mode)
        .corner_radius(8.0)
        .min_size(egui::vec2(38.0, 32.0));
      if ui.add(button).on_hover_text(tooltip).clicked() {
        apply_mode(app, mode);
      }
    }

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);

    let simulate_button = egui::Button::new(icons::icon("play", 17.0))
      .selected(app.simulate_open)
      .corner_radius(8.0)
      .min_size(egui::vec2(38.0, 32.0));
    if ui.add(simulate_button).on_hover_text("Simular").clicked() {
      toggle_simulate(app);
    }
  });
}

pub(crate) fn token_chip(ui: &mut egui::Ui, place_label: &str, tokens: u32) {
  egui::Frame::default()
    .fill(ui.visuals().extreme_bg_color)
    .corner_radius(10.0)
    .inner_margin(egui::Margin::symmetric(8, 3))
    .show(ui, |ui| {
      // Extend, not the `horizontal_wrapped` default Wrap: otherwise long text wraps letter by
      // letter inside the chip instead of the whole chip moving to the next row.
      ui.add(
        egui::Label::new(egui::RichText::new(format!("{place_label}  {tokens}")).monospace())
          .wrap_mode(egui::TextWrapMode::Extend),
      );
    });
}

/// One chip per marked place, wrapped onto new rows as needed. The one place that renders a
/// full `Marking` as chips instead of a flat comma-joined string, so each label/count pair stays
/// visually paired even when it wraps.
pub(crate) fn marking_chips(
  ui: &mut egui::Ui,
  net: &crate::model::PetriNet,
  marking: &crate::model::Marking,
) {
  ui.horizontal_wrapped(|ui| {
    let mut any = false;
    for (&place, &tokens) in marking.iter() {
      if tokens > 0 {
        any = true;
        token_chip(ui, net.place_label(place), tokens);
      }
    }
    if !any {
      ui.weak("(vacío)");
    }
  });
}

/// Step-through simulator: shows the live net's current marking as token chips, its enabled
/// transitions as one-click fire buttons, and back/reset/forward controls over an undo/redo
/// history. It's the same firing rule the reachability graph explores and the canvas
/// token-game uses — this is just a docked, friendlier way to drive it.
pub fn simulate_panel(app: &mut PetriApp, ui: &mut egui::Ui) {
  // A fixed (not max) width keeps every row's `available_width()` identical across frames;
  // `set_max_width` let the full-width "fire" button and the control row disagree on width
  // between passes, which is what made the whole popup render off-center.
  ui.set_width(280.0);
  ui.horizontal(|ui| {
    ui.label(icons::icon("play", 14.0));
    ui.strong("Simulación");
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
      if ui
        .add(egui::Button::new(icons::icon("x", 13.0)).frame(false))
        .on_hover_text("Cerrar simulación")
        .clicked()
      {
        toggle_simulate(app);
      }
    });
  });
  ui.weak(format!(
    "Paso {} · {} adelante disponible{}",
    app.sim_history.len(),
    app.sim_future.len(),
    if app.sim_future.len() == 1 { "" } else { "s" }
  ));
  ui.add_space(12.0);

  let ctrl_btn = |icon_name: &'static str| {
    egui::Button::new(icons::icon(icon_name, 15.0))
      .corner_radius(6.0)
      .min_size(egui::vec2(36.0, 30.0))
  };
  ui.vertical_centered(|ui| {
    ui.horizontal(|ui| {
      let can_back = !app.sim_history.is_empty();
      let can_forward = !app.sim_future.is_empty();
      let can_reset = app.sim_initial.is_some();

      if ui
        .add_enabled(can_back, ctrl_btn("skip-back"))
        .on_hover_text("Paso atrás")
        .clicked()
      {
        step_back(app);
      }
      if ui
        .add_enabled(can_reset, ctrl_btn("rotate-ccw"))
        .on_hover_text("Reiniciar")
        .clicked()
      {
        reset_sim(app);
      }
      if ui
        .add_enabled(can_forward, ctrl_btn("skip-forward"))
        .on_hover_text("Paso adelante")
        .clicked()
      {
        step_forward(app);
      }
    });
  });
  ui.add_space(14.0);

  let marking = app.net.marking();
  let total_tokens: u32 = marking.values().sum();
  section_label(ui, &format!("Marking · {total_tokens} tokens"));
  ui.add_space(6.0);
  card(ui, |ui| {
    marking_chips(ui, &app.net, &marking);
  });
  ui.add_space(12.0);

  let enabled = fire::enabled_transitions(&app.net, &marking);
  section_label(ui, &format!("Transiciones habilitadas · {}", enabled.len()));
  ui.add_space(6.0);
  card(ui, |ui| {
    if enabled.is_empty() {
      ui.weak("(ninguna)");
    } else {
      for t in enabled {
        let btn = egui::Button::new((
          icons::icon("zap", 13.0),
          app.net.transition_label(t).to_string(),
        ))
        .corner_radius(6.0);
        if ui.add_sized([ui.available_width(), 28.0], btn).clicked() {
          fire_step(app, t);
        }
      }
    }
  });
}

#[cfg(test)]
mod ux_tests {
  use super::*;

  fn place_at(app: &mut PetriApp, x: f32, y: f32) -> NodeId {
    let id = app.net.add_place("p");
    let node = NodeId::Place(id);
    app.positions.insert(node, egui::pos2(x, y));
    node
  }

  #[test]
  fn snap_to_grid_rounds_to_nearest_step() {
    assert_eq!(snap_to_grid(egui::pos2(10.0, 10.0)), egui::pos2(0.0, 0.0));
    assert_eq!(snap_to_grid(egui::pos2(13.0, 40.0)), egui::pos2(24.0, 48.0));
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
  fn align_auto_levels_a_wide_group_into_a_row() {
    let mut app = PetriApp::new();
    // Wider than tall: auto should level the y's (turn it into a row), not the x's.
    let a = place_at(&mut app, 0.0, 0.0);
    let b = place_at(&mut app, 100.0, 20.0);
    align_selected(&mut app, &HashSet::from([a, b]), Align::Auto);
    assert_eq!(app.positions[&a].y, app.positions[&b].y);
    assert_eq!(app.positions[&a].x, 0.0); // x left untouched by a row-align
  }

  #[test]
  fn undo_redo_roundtrips_a_checkpointed_change() {
    let mut app = PetriApp::new();
    checkpoint(&mut app);
    let id = app.net.add_place("p1");
    assert_eq!(app.net.place_ids().count(), 1);

    undo(&mut app);
    assert_eq!(app.net.place_ids().count(), 0);

    redo(&mut app);
    assert_eq!(app.net.place_ids().count(), 1);
    assert_eq!(app.net.place_label(id), "p1");
  }

  #[test]
  fn copy_paste_duplicates_selection_and_its_internal_arc() {
    let mut app = PetriApp::new();
    let p = app.net.add_place("p");
    let t = app.net.add_transition("t");
    app
      .net
      .add_arc_place_to_transition(p, t, ArcKind::Consume(1))
      .unwrap();
    app.positions.insert(NodeId::Place(p), egui::pos2(0.0, 0.0));
    app.positions.insert(NodeId::Transition(t), egui::pos2(50.0, 0.0));
    app.selection = Selection::Nodes(HashSet::from([NodeId::Place(p), NodeId::Transition(t)]));

    copy_selection(&mut app);
    paste_clipboard(&mut app);

    assert_eq!(app.net.place_ids().count(), 2);
    assert_eq!(app.net.transition_ids().count(), 2);
    let new_t = app.net.transition_ids().find(|&x| x != t).unwrap();
    assert_eq!(app.net.inputs(new_t).len(), 1); // the copied arc came along with it

    let Selection::Nodes(sel) = &app.selection else {
      panic!("paste should leave the new nodes selected")
    };
    assert_eq!(sel.len(), 2);
  }
}
