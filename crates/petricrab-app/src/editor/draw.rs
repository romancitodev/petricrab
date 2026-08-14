use std::collections::HashSet;

use eframe::egui;

use crate::app::{NodeId, PetriApp, Selection};
use crate::model::fire;
use crate::theme;

use super::geometry::{
  ArcEnd, GRID_SPACING, PLACE_RADIUS, T_HALF_H, T_HALF_W, arc_end_for_kind, boundary_margin,
  compatible, dist_to_segment, hit_test, node_pos, rounded_rect_polygon, to_screen,
  transition_angle,
};

const HALO_PAD: f32 = 4.0;
const ARC_CURVE_SEGMENTS: usize = 16;
const MAX_ARROWHEADS: u32 = 4;
const INHIBIT_DASH_LEN: f32 = 5.0;
const INHIBIT_GAP_LEN: f32 = 4.0;
/// Extra clearance (beyond a node's own footprint) an arc tries to keep from any node it's not
/// actually connecting, before it's considered "in the way" and worth bowing around.
const ARC_OBSTACLE_PADDING: f32 = 14.0;
/// World-space arc length past which a straight, unobstructed run still gets a gentle bow — a
/// dead-straight long line reads as noise on a busy canvas even with nothing in its way.
const ARC_LONG_THRESHOLD: f32 = 260.0;

/// A dot at every grid intersection instead of full lines.
pub(crate) fn draw_grid(painter: &egui::Painter, rect: egui::Rect, pan: egui::Vec2, zoom: f32) {
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
pub(crate) fn arc_bow(
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
pub(crate) fn dist_to_arc(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2, bow: f32) -> f32 {
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
pub(crate) fn draw_selection_halo(
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

pub(crate) fn draw_marquee(
  painter: &egui::Painter,
  a: egui::Pos2,
  b: egui::Pos2,
  accent: egui::Color32,
) {
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

pub(crate) fn draw_net(
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
pub(crate) fn note_rect(note: &crate::app::NoteData) -> egui::Rect {
  egui::Rect::from_min_size(note.pos, note.size)
}

/// Free-form text annotations — a small card per note. The selected one's text is edited live
/// via an actual `TextEdit` placed on top (see `note_edit_overlay`), so its static text is
/// skipped here to avoid drawing under the widget; every other note gets wrapped, clipped
/// static text painted directly (cheaper than a widget for something you're not touching).
pub(crate) fn draw_notes(
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
pub(crate) fn note_edit_overlay(app: &mut PetriApp, ui: &mut egui::Ui, pan: egui::Vec2, zoom: f32) {
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
pub(crate) fn draw_connect_preview(
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
