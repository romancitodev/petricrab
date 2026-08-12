use std::collections::HashSet;

use crate::model::{ArcKind, fire};
use eframe::egui;

use crate::app::{ContextTarget, EditMode, NodeId, PetriApp, Selection};
use crate::icons;
use crate::theme;

const PLACE_RADIUS: f32 = 24.0;
const T_HALF_W: f32 = 6.0;
const T_HALF_H: f32 = 26.0;
const GRID_SPACING: f32 = 24.0;
const ARC_HIT_DIST: f32 = 6.0;
const HALO_PAD: f32 = 6.0;
const ARC_CURVE_SEGMENTS: usize = 16;
const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 3.0;
const MAX_ARROWHEADS: u32 = 4;

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

#[derive(Clone, Copy)]
enum ArcEnd {
  Arrow,
  Diamond,
  Circle,
}

fn arc_end_for_kind(kind: ArcKind) -> ArcEnd {
  match kind {
    ArcKind::Consume(_) => ArcEnd::Arrow,
    ArcKind::Peek(_) => ArcEnd::Diamond,
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

/// The transition's two (rotated) half-extent axes. `center ± right ± up` gives its four
/// corners, in whatever space `half_w`/`half_h` are already expressed in.
fn transition_axes(
  app: &PetriApp,
  t: crate::model::TransitionId,
  half_w: f32,
  half_h: f32,
) -> (egui::Vec2, egui::Vec2) {
  let angle = transition_angle(app, t);
  (
    rotate_vec(egui::vec2(half_w, 0.0), angle),
    rotate_vec(egui::vec2(0.0, half_h), angle),
  )
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
      if dist_to_arc(pos, p_pos, t_pos, is_reciprocal(&app.net, p, t)) <= ARC_HIT_DIST {
        return Some(Selection::ArcIn(p, t));
      }
    }
    for &(p, _weight) in app.net.outputs(t) {
      let Some(p_pos) = app.positions.get(&NodeId::Place(p)).copied() else {
        continue;
      };
      if dist_to_arc(pos, t_pos, p_pos, is_reciprocal(&app.net, p, t)) <= ARC_HIT_DIST {
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
  let c = theme::TEXT;
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
/// Straight (`curved == false`) by default. Only a place<->transition pair with arcs running
/// both directions (`is_reciprocal`) needs the bow, to keep the two from overlapping.
fn arc_control_point(a: egui::Pos2, b: egui::Pos2, curved: bool) -> egui::Pos2 {
  let delta = b - a;
  let len = delta.length();
  if len < 1.0 || !curved {
    return a + delta * 0.5;
  }
  let dir = delta / len;
  let normal = egui::vec2(-dir.y, dir.x);
  let bow = (len * 0.15).min(36.0);
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

/// World-space distance from `p` to the curved arc between world-space centers `a`/`b`.
fn dist_to_arc(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2, curved: bool) -> f32 {
  let control = arc_control_point(a, b, curved);
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
  curved: bool,
  stroke: egui::Stroke,
  pan: egui::Vec2,
  zoom: f32,
  background: egui::Color32,
) {
  let delta = to - from;
  if delta.length() < 1.0 {
    return;
  }
  let control = arc_control_point(from, to, curved);

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
  painter.add(egui::Shape::line(points, stroke));

  let dir = end_dir;
  let normal = egui::vec2(-dir.y, dir.x);
  let reps = weight.clamp(1, MAX_ARROWHEADS);
  for i in 0..reps {
    let offset = dir * (i as f32 * 7.0);
    let tip = to_edge - offset;
    match end {
      ArcEnd::Arrow => {
        // A touch longer/narrower than a stock triangle so it reads as a sleek chevron instead
        // of a blunt flag, at both toolbar-icon and zoomed-out scale.
        let back = tip - dir * 11.0;
        painter.add(egui::Shape::convex_polygon(
          vec![ts(tip), ts(back + normal * 3.4), ts(back - normal * 3.4)],
          stroke.color,
          egui::Stroke::NONE,
        ));
      }
      ArcEnd::Diamond => {
        let back = tip - dir * 14.0;
        let mid = tip - dir * 7.0;
        painter.add(egui::Shape::convex_polygon(
          vec![
            ts(tip),
            ts(mid + normal * 5.0),
            ts(back),
            ts(mid - normal * 5.0),
          ],
          egui::Color32::TRANSPARENT,
          stroke,
        ));
      }
      ArcEnd::Circle => {
        // Hollow ring, not a filled dot: matches the classic inhibitor-arc notation (a filled
        // circle would read as a token/peek marker instead) and keeps the endpoint light on a
        // busy canvas.
        let center = tip - dir * 4.5;
        painter.circle_filled(ts(center), 4.5 * zoom, background);
        painter.circle_stroke(
          ts(center),
          4.5 * zoom,
          egui::Stroke::new(stroke.width, stroke.color),
        );
      }
    }
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
  let fill = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 55);
  let stroke = egui::Stroke::new(2.0 * zoom, accent);
  match node {
    NodeId::Place(_) => {
      painter.circle_filled(pos, (PLACE_RADIUS + HALO_PAD) * zoom, fill);
      painter.circle_stroke(pos, (PLACE_RADIUS + HALO_PAD) * zoom, stroke);
    }
    NodeId::Transition(t) if transition_angle(app, t) == 0.0 => {
      let r = egui::Rect::from_center_size(
        pos,
        egui::vec2((T_HALF_W + HALO_PAD) * 2.0, (T_HALF_H + HALO_PAD) * 2.0) * zoom,
      );
      let radius = (T_HALF_W + HALO_PAD) * zoom;
      painter.rect_filled(r, radius, fill);
      painter.rect_stroke(r, radius, stroke, egui::StrokeKind::Outside);
    }
    NodeId::Transition(t) => {
      let (right, up) = transition_axes(
        app,
        t,
        (T_HALF_W + HALO_PAD) * zoom,
        (T_HALF_H + HALO_PAD) * zoom,
      );
      painter.add(egui::Shape::convex_polygon(
        vec![
          pos + right + up,
          pos - right + up,
          pos - right - up,
          pos + right - up,
        ],
        fill,
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
  let arc_stroke = egui::Stroke::new(1.2, theme::TEXT_WEAK);
  let accent = visuals.selection.bg_fill;

  // Arcs (under nodes).
  for t in app.net.transition_ids() {
    let t_pos = node_pos(app, NodeId::Transition(t), fallback);
    for &(p, kind) in app.net.inputs(t) {
      let p_pos = node_pos(app, NodeId::Place(p), fallback);
      draw_arc(
        app,
        painter,
        p_pos,
        t_pos,
        NodeId::Place(p),
        NodeId::Transition(t),
        arc_end_for_kind(kind),
        kind.weight(),
        is_reciprocal(&app.net, p, t),
        arc_stroke,
        pan,
        zoom,
        visuals.extreme_bg_color,
      );
    }
    for &(p, weight) in app.net.outputs(t) {
      let p_pos = node_pos(app, NodeId::Place(p), fallback);
      draw_arc(
        app,
        painter,
        t_pos,
        p_pos,
        NodeId::Transition(t),
        NodeId::Place(p),
        ArcEnd::Arrow,
        weight,
        is_reciprocal(&app.net, p, t),
        arc_stroke,
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

  // Places: same fill as the canvas itself, just a thin crisp ring. Token dots are the only
  // solid white in the shape.
  for p in app.net.place_ids() {
    let pos = s(node_pos(app, NodeId::Place(p), fallback));
    painter.circle_filled(pos, PLACE_RADIUS * zoom, theme::INK);
    painter.circle_stroke(
      pos,
      PLACE_RADIUS * zoom,
      egui::Stroke::new(1.4 * zoom, theme::TEXT),
    );
    draw_tokens(painter, pos, app.net.tokens(p), zoom, theme::TEXT_STRONG);
    painter.text(
      pos + egui::vec2(0.0, (PLACE_RADIUS + 12.0) * zoom),
      egui::Align2::CENTER_CENTER,
      app.net.place_label(p),
      egui::FontId::proportional(12.0 * zoom),
      visuals.weak_text_color(),
    );
  }

  // Axis-aligned bars use the fast rounded-rect path; a rotated transition draws as an explicit
  // polygon instead, since egui has no rotated-rounded-rect primitive.
  for t in app.net.transition_ids() {
    let pos = s(node_pos(app, NodeId::Transition(t), fallback));
    let angle = transition_angle(app, t);
    let fill = if enabled.contains(&t) {
      theme::SUCCESS
    } else {
      theme::TEXT_STRONG
    };
    let label_y = if angle == 0.0 {
      let r = egui::Rect::from_center_size(pos, egui::vec2(T_HALF_W * 2.0, T_HALF_H * 2.0) * zoom);
      // A radius of half the short side turns the rounded rect into a full capsule/pill.
      painter.rect_filled(r, T_HALF_W * zoom, fill);
      r.bottom()
    } else {
      let (right, up) = transition_axes(app, t, T_HALF_W * zoom, T_HALF_H * zoom);
      let corners = [
        pos + right + up,
        pos - right + up,
        pos - right - up,
        pos + right - up,
      ];
      painter.add(egui::Shape::convex_polygon(
        corners.to_vec(),
        fill,
        egui::Stroke::NONE,
      ));
      corners.iter().map(|c| c.y).fold(f32::MIN, f32::max)
    };
    painter.text(
      egui::pos2(pos.x, label_y + 12.0 * zoom),
      egui::Align2::CENTER_CENTER,
      app.net.transition_label(t),
      egui::FontId::proportional(12.0 * zoom),
      visuals.weak_text_color(),
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
          is_reciprocal(&app.net, p, t),
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
        is_reciprocal(&app.net, p, t),
        egui::Stroke::new(3.0, accent),
        pan,
        zoom,
        visuals.extreme_bg_color,
      );
    }
    _ => {}
  }
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
        false,
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
    Selection::None => {}
  }
}

/// `pos` is in world space.
fn handle_click(app: &mut PetriApp, pos: egui::Pos2) {
  let hit = hit_test(app, pos);
  match app.mode {
    EditMode::AddPlace => {
      if hit.is_none() {
        app.next_place_n += 1;
        let id = app.net.add_place(format!("p{}", app.next_place_n));
        app.positions.insert(NodeId::Place(id), pos);
      }
    }
    EditMode::AddTransition => {
      if hit.is_none() {
        app.next_transition_n += 1;
        let id = app
          .net
          .add_transition(format!("t{}", app.next_transition_n));
        app.positions.insert(NodeId::Transition(id), pos);
      }
    }
    EditMode::Connect => match hit {
      Some(node) => match app.connect_from.take() {
        None => app.connect_from = Some(node),
        Some(from) => connect(app, from, node),
      },
      None => app.connect_from = None,
    },
    EditMode::Select => match hit {
      Some(node) => {
        app.selection = Selection::Nodes(HashSet::from([node]));
        if let NodeId::Transition(t) = node {
          fire_step(app, t);
        }
      }
      None => {
        app.selection = hit_test_arc(app, pos).unwrap_or(Selection::None);
      }
    },
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
  let pan = app.pan;
  let zoom = app.zoom;
  let fallback = to_world(rect.center(), pan, zoom);

  // CentralPanel has no fill of its own here, so paint the canvas background explicitly.
  painter.rect_filled(rect, 0.0, theme::INK);
  if app.show_grid {
    draw_grid(&painter, rect, pan, zoom);
  }
  draw_net(app, &painter, fallback, pan, zoom, &visuals);

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

  if !space_held {
    if response.drag_started() {
      if let Some(pos) = response.interact_pointer_pos() {
        let world = to_world(pos, pan, zoom);
        app.dragging = hit_test(app, world);
        if app.dragging.is_none() && app.mode == EditMode::Select {
          app.marquee_start = Some(world);
          app.marquee_current = Some(world);
        }
      }
    }
    if response.dragged() {
      if let Some(node) = app.dragging {
        let delta = response.drag_delta() / zoom;
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
      if i.key_pressed(egui::Key::V) {
        apply_mode(app, EditMode::Select);
      }
      if i.key_pressed(egui::Key::P) {
        apply_mode(app, EditMode::AddPlace);
      }
      if i.key_pressed(egui::Key::T) {
        apply_mode(app, EditMode::AddTransition);
      }
      if i.key_pressed(egui::Key::C) {
        apply_mode(app, EditMode::Connect);
      }
    });
  }
}

fn reset_view(app: &mut PetriApp) {
  app.pan = egui::Vec2::ZERO;
  app.zoom = 1.0;
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

fn menu_item(ui: &mut egui::Ui, icon_name: &'static str, label: &str) -> egui::Response {
  ui.add(egui::Button::new((icons::icon(icon_name, 13.0), label)).corner_radius(6.0))
}

fn danger_menu_item(ui: &mut egui::Ui, icon_name: &'static str, label: &str) -> egui::Response {
  ui.add(
    egui::Button::new((
      icons::icon(icon_name, 13.0).color(theme::DANGER),
      egui::RichText::new(label).color(theme::DANGER),
    ))
    .corner_radius(6.0),
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
          egui::Button::new((icons::icon("minus", 13.0), "Quitar token")).corner_radius(6.0),
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
          egui::Button::new((icons::icon("x", 13.0), "Vaciar tokens")).corner_radius(6.0),
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
          egui::Button::new((icons::icon("zap", 13.0), "Disparar")).corner_radius(6.0),
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
            egui::Button::new((icons::icon("minus", 13.0), "Disminuir peso")).corner_radius(6.0),
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
            .corner_radius(6.0),
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
      app.next_place_n = loaded.next_place_n;
      app.next_transition_n = loaded.next_transition_n;
      app.notify(
        egui_toast::ToastKind::Success,
        format!("Abierto: {}", file_display_name(&path)),
      );
      remember_recent(app, path.clone());
      app.file_path = Some(path);
    }
    Err(e) => app.notify(
      egui_toast::ToastKind::Error,
      format!("No se pudo abrir el proyecto: {e}"),
    ),
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
      app.notify(
        egui_toast::ToastKind::Success,
        format!("Guardado: {}", file_display_name(&path)),
      );
      remember_recent(app, path.clone());
      app.file_path = Some(path);
    }
    Err(e) => app.notify(
      egui_toast::ToastKind::Error,
      format!("No se pudo guardar el proyecto: {e}"),
    ),
  }
}

fn file_save(app: &mut PetriApp) {
  match app.file_path.clone() {
    Some(path) => match crate::project::save(app, &path) {
      Ok(()) => {
        app.notify(
          egui_toast::ToastKind::Success,
          format!("Guardado: {}", file_display_name(&path)),
        );
        remember_recent(app, path);
      }
      Err(e) => app.notify(
        egui_toast::ToastKind::Error,
        format!("No se pudo guardar el proyecto: {e}"),
      ),
    },
    None => file_save_as(app),
  }
}

/// Top menu bar: identity mark on the left, then File/Edit/View menus.
pub fn menu_bar(app: &mut PetriApp, ui: &mut egui::Ui, ctx: &egui::Context) {
  egui::MenuBar::new().ui(ui, |ui| {
    ui.horizontal(|ui| {
      ui.label(icons::icon("workflow", 16.0).color(theme::ACCENT));
      ui.add_space(2.0);
      ui.label(egui::RichText::new("petricrab").strong().size(14.0));
    });
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(4.0);

    ui.menu_button("Archivo", |ui| {
      ui.set_min_width(170.0);
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
      ui.add_enabled(
        false,
        egui::Button::new((icons::icon("undo-2", 13.0), "Deshacer")).corner_radius(6.0),
      )
      .on_disabled_hover_text("Próximamente");
      ui.add_enabled(
        false,
        egui::Button::new((icons::icon("redo-2", 13.0), "Rehacer")).corner_radius(6.0),
      )
      .on_disabled_hover_text("Próximamente");
    });

    ui.menu_button("Ver", |ui| {
      ui.set_min_width(170.0);
      ui.checkbox(&mut app.show_grid, "Mostrar grilla");
      if menu_item(ui, "locate-fixed", "Reiniciar vista").clicked() {
        reset_view(app);
        ui.close();
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

/// Configuration for a floating analysis window — see [`floating_window`].
pub(crate) struct WindowSpec {
  pub id: &'static str,
  pub icon: &'static str,
  pub title: &'static str,
  pub default_size: egui::Vec2,
  pub min_size: egui::Vec2,
  pub max_size: Option<egui::Vec2>,
  pub movable: bool,
}

/// Shared chrome for every floating analysis window (reachability graph, properties, future
/// ones): an icon + title header instead of a bare label, no collapse-to-titlebar affordance
/// (these are dedicated panels, not generic inspectors you tuck away), and one consistent
/// frame/shadow/corner-radius instead of each window re-deriving its own.
///
/// Returns the window's on-screen rect this frame (`None` if it wasn't drawn, e.g. collapsed to
/// nothing) — callers that embed something position-sensitive (egui_graphs' reachability graph)
/// need this to detect the window moving; see `ReachabilityState::note_window_moved`.
pub(crate) fn floating_window(
  ctx: &egui::Context,
  visuals: &egui::Visuals,
  spec: WindowSpec,
  open: &mut bool,
  add_contents: impl FnOnce(&mut egui::Ui),
) -> Option<egui::Rect> {
  let mut window = egui::Window::new((icons::icon(spec.icon, 15.0), spec.title))
    .id(egui::Id::new(spec.id))
    .frame(
      egui::Frame::default()
        .fill(visuals.panel_fill)
        .stroke(egui::Stroke::new(1.0, visuals.window_stroke.color))
        .corner_radius(theme::RADIUS_LG)
        .shadow(visuals.window_shadow)
        .inner_margin(egui::Margin::symmetric(16, 14)),
    )
    .default_size(spec.default_size)
    .min_size(spec.min_size)
    .resizable(true)
    .collapsible(false)
    .movable(spec.movable)
    .open(open);
  if let Some(max_size) = spec.max_size {
    window = window.max_size(max_size);
  }
  window
    .show(ctx, add_contents)
    .map(|inner| inner.response.rect)
}

fn destructive_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
  let danger = theme::DANGER;
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
    _ => {
      ui.weak("Nada seleccionado");
    }
  }
}

fn mode_icon_and_tooltip(mode: EditMode) -> (&'static str, &'static str) {
  match mode {
    EditMode::Select => ("mouse-pointer-2", "Seleccionar / token game (V)"),
    EditMode::AddPlace => ("circle", "Agregar place (P)"),
    EditMode::AddTransition => ("rectangle-vertical", "Agregar transition (T)"),
    EditMode::Connect => ("cable", "Conectar arco (C)"),
  }
}

pub fn toolbar(app: &mut PetriApp, ui: &mut egui::Ui) {
  ui.horizontal(|ui| {
    for mode in [
      EditMode::Select,
      EditMode::AddPlace,
      EditMode::AddTransition,
      EditMode::Connect,
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
      ui.label(egui::RichText::new(format!("{place_label}  {tokens}")).monospace());
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
  });
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

  section_label(ui, "Marking");
  ui.add_space(6.0);
  card(ui, |ui| {
    let has_tokens = app.net.place_ids().any(|p| app.net.tokens(p) > 0);
    if has_tokens {
      ui.horizontal_wrapped(|ui| {
        for p in app.net.place_ids() {
          let tokens = app.net.tokens(p);
          if tokens > 0 {
            token_chip(ui, app.net.place_label(p), tokens);
          }
        }
      });
    } else {
      ui.weak("(vacío)");
    }
  });
  ui.add_space(12.0);

  section_label(ui, "Transiciones habilitadas");
  ui.add_space(6.0);
  card(ui, |ui| {
    let marking = app.net.marking();
    let enabled = fire::enabled_transitions(&app.net, &marking);
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
