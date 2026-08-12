use std::collections::HashSet;

use eframe::egui;
use crate::model::{fire, ArcKind};

use crate::app::{EditMode, NodeId, PetriApp, Selection};
use crate::icons;

const PLACE_RADIUS: f32 = 24.0;
const T_HALF_W: f32 = 6.0;
const T_HALF_H: f32 = 26.0;
const T_ROUNDING: f32 = 3.0;
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

/// Distance from the node's center to its boundary along `dir` (unit vector, pointing away
/// from the node), in world units. Places are circles (constant radius); transitions are
/// axis-aligned rectangles, so the boundary distance depends on the approach angle.
fn boundary_margin(node: NodeId, dir: egui::Vec2) -> f32 {
    match node {
        NodeId::Place(_) => PLACE_RADIUS,
        NodeId::Transition(_) => {
            let dx = if dir.x.abs() > 1e-4 {
                T_HALF_W / dir.x.abs()
            } else {
                f32::INFINITY
            };
            let dy = if dir.y.abs() > 1e-4 {
                T_HALF_H / dir.y.abs()
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
            NodeId::Transition(_) => {
                (pos.x - node_pos.x).abs() <= T_HALF_W && (pos.y - node_pos.y).abs() <= T_HALF_H
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
            if dist_to_arc(pos, p_pos, t_pos) <= ARC_HIT_DIST {
                return Some(Selection::ArcIn(p, t));
            }
        }
        for &(p, _weight) in app.net.outputs(t) {
            let Some(p_pos) = app.positions.get(&NodeId::Place(p)).copied() else {
                continue;
            };
            if dist_to_arc(pos, t_pos, p_pos) <= ARC_HIT_DIST {
                return Some(Selection::ArcOut(t, p));
            }
        }
    }
    None
}

/// `rect` is in world space.
fn nodes_in_rect(app: &PetriApp, rect: egui::Rect) -> HashSet<NodeId> {
    app.positions
        .iter()
        .filter(|&(_, &pos)| rect.contains(pos))
        .map(|(&node, _)| node)
        .collect()
}

fn draw_grid(painter: &egui::Painter, rect: egui::Rect, pan: egui::Vec2, zoom: f32, visuals: &egui::Visuals) {
    let c = visuals.text_color();
    let line = egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 14));
    let spacing = GRID_SPACING * zoom;
    if spacing < 4.0 {
        return;
    }

    let mut x = (((rect.left() - pan.x) / spacing).floor() * spacing) + pan.x;
    while x < rect.right() {
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], line);
        x += spacing;
    }
    let mut y = (((rect.top() - pan.y) / spacing).floor() * spacing) + pan.y;
    while y < rect.bottom() {
        painter.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], line);
        y += spacing;
    }
}

/// World-space quadratic-bezier control point for the arc between two world-space centers:
/// bows gently to one side so arcs read as flexible connectors instead of rigid rulers, and so
/// that a place->transition arc and its transition->place sibling (opposite `dir`) naturally
/// separate onto different sides instead of overlapping.
fn arc_control_point(a: egui::Pos2, b: egui::Pos2) -> egui::Pos2 {
    let delta = b - a;
    let len = delta.length();
    if len < 1.0 {
        return a;
    }
    let dir = delta / len;
    let normal = egui::vec2(-dir.y, dir.x);
    let bow = (len * 0.15).min(36.0);
    a + delta * 0.5 + normal * bow
}

/// World-space distance from `p` to the curved arc between world-space centers `a`/`b`.
fn dist_to_arc(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let control = arc_control_point(a, b);
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
) {
    let delta = to - from;
    if delta.length() < 1.0 {
        return;
    }
    let control = arc_control_point(from, to);

    // Trim the curve to the node boundaries using the direction at each endpoint (tangent for
    // the transition end, straight line for the start; close enough for a subtle bow).
    let start_dir = (control - from).normalized();
    let end_dir = (to - control).normalized();
    let from_edge = from + start_dir * boundary_margin(from_node, start_dir);
    let to_edge = to - end_dir * boundary_margin(to_node, -end_dir);

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
                let back = tip - dir * 10.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![ts(tip), ts(back + normal * 4.0), ts(back - normal * 4.0)],
                    stroke.color,
                    egui::Stroke::NONE,
                ));
            }
            ArcEnd::Diamond => {
                let back = tip - dir * 14.0;
                let mid = tip - dir * 7.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![ts(tip), ts(mid + normal * 5.0), ts(back), ts(mid - normal * 5.0)],
                    egui::Color32::TRANSPARENT,
                    stroke,
                ));
            }
            ArcEnd::Circle => {
                painter.circle_filled(ts(tip - dir * 4.0), 4.0 * zoom, stroke.color);
            }
        }
    }
}

/// Draws token count as dots (classic Petri-net notation) for small counts, falling back to a
/// number once dots would get too crowded to read at a glance. `center` is in screen space.
fn draw_tokens(painter: &egui::Painter, center: egui::Pos2, tokens: u32, zoom: f32, color: egui::Color32) {
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
fn draw_selection_halo(painter: &egui::Painter, node: NodeId, pos: egui::Pos2, zoom: f32, accent: egui::Color32) {
    let fill = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 55);
    let stroke = egui::Stroke::new(2.0 * zoom, accent);
    match node {
        NodeId::Place(_) => {
            painter.circle_filled(pos, (PLACE_RADIUS + HALO_PAD) * zoom, fill);
            painter.circle_stroke(pos, (PLACE_RADIUS + HALO_PAD) * zoom, stroke);
        }
        NodeId::Transition(_) => {
            let r = egui::Rect::from_center_size(
                pos,
                egui::vec2((T_HALF_W + HALO_PAD) * 2.0, (T_HALF_H + HALO_PAD) * 2.0) * zoom,
            );
            let rounding = (T_ROUNDING + 3.0) * zoom;
            painter.rect_filled(r, rounding, fill);
            painter.rect_stroke(r, rounding, stroke, egui::StrokeKind::Outside);
        }
    }
}

fn draw_marquee(painter: &egui::Painter, a: egui::Pos2, b: egui::Pos2, accent: egui::Color32) {
    let rect = egui::Rect::from_two_pos(a, b);
    let fill = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 28);
    painter.rect_filled(rect, 2.0, fill);
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, accent), egui::StrokeKind::Outside);
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
    let arc_color = visuals.text_color();
    let arc_stroke = egui::Stroke::new(1.6, arc_color);
    let accent = visuals.selection.bg_fill;

    // Arcs (under nodes).
    for t in app.net.transition_ids() {
        let t_pos = node_pos(app, NodeId::Transition(t), fallback);
        for &(p, kind) in app.net.inputs(t) {
            let p_pos = node_pos(app, NodeId::Place(p), fallback);
            draw_arc(
                painter,
                p_pos,
                t_pos,
                NodeId::Place(p),
                NodeId::Transition(t),
                arc_end_for_kind(kind),
                kind.weight(),
                arc_stroke,
                pan,
                zoom,
            );
        }
        for &(p, weight) in app.net.outputs(t) {
            let p_pos = node_pos(app, NodeId::Place(p), fallback);
            draw_arc(
                painter,
                t_pos,
                p_pos,
                NodeId::Transition(t),
                NodeId::Place(p),
                ArcEnd::Arrow,
                weight,
                arc_stroke,
                pan,
                zoom,
            );
        }
    }

    // Selection halos (under the nodes they highlight, over the arcs).
    if let Selection::Nodes(nodes) = &app.selection {
        for &node in nodes {
            draw_selection_halo(painter, node, s(node_pos(app, node, fallback)), zoom, accent);
        }
    }

    // Places.
    for p in app.net.place_ids() {
        let pos = s(node_pos(app, NodeId::Place(p), fallback));
        painter.circle_filled(pos, PLACE_RADIUS * zoom, visuals.extreme_bg_color);
        painter.circle_stroke(pos, PLACE_RADIUS * zoom, egui::Stroke::new(1.8 * zoom, visuals.text_color()));
        draw_tokens(painter, pos, app.net.tokens(p), zoom, visuals.text_color());
        painter.text(
            pos + egui::vec2(0.0, (PLACE_RADIUS + 12.0) * zoom),
            egui::Align2::CENTER_CENTER,
            app.net.place_label(p),
            egui::FontId::proportional(12.0 * zoom),
            visuals.weak_text_color(),
        );
    }

    // Transitions.
    for t in app.net.transition_ids() {
        let pos = s(node_pos(app, NodeId::Transition(t), fallback));
        let r = egui::Rect::from_center_size(pos, egui::vec2(T_HALF_W * 2.0, T_HALF_H * 2.0) * zoom);
        let fill = if enabled.contains(&t) {
            egui::Color32::from_rgb(70, 165, 95)
        } else {
            visuals.strong_text_color()
        };
        painter.rect_filled(r, T_ROUNDING * zoom, fill);
        painter.text(
            pos + egui::vec2(0.0, (T_HALF_H + 12.0) * zoom),
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
            let kind = app.net.inputs(t).iter().find(|(place, _)| *place == p).map(|&(_, k)| k);
            if let Some(kind) = kind {
                draw_arc(
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
                );
            }
        }
        Selection::ArcOut(t, p) => {
            let (t, p) = (*t, *p);
            let t_pos = node_pos(app, NodeId::Transition(t), fallback);
            let p_pos = node_pos(app, NodeId::Place(p), fallback);
            let weight = app.net.outputs(t).iter().find(|(place, _)| *place == p).map(|&(_, w)| w).unwrap_or(1);
            draw_arc(
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
    let dimmed = egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 100));

    match hovered {
        Some(target) => {
            let target_pos = node_pos(app, target, fallback);
            draw_selection_halo(painter, target, s(target_pos), zoom, visuals.selection.bg_fill);
            draw_arc(painter, from_pos, target_pos, from, target, ArcEnd::Arrow, 1, dimmed, pan, zoom);
        }
        None => {
            // No boundary_margin on the free end: it's just the mouse position, not a node.
            let delta = mouse - from_pos;
            if delta.length() >= 1.0 {
                let dir = delta.normalized();
                let from_edge = from_pos + dir * boundary_margin(from, dir);
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
            app.net.add_arc_place_to_transition(p, t, ArcKind::Consume(1))
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
                let id = app.net.add_transition(format!("t{}", app.next_transition_n));
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
    let (response, painter) =
        ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
    let rect = response.rect;
    let pan = app.pan;
    let zoom = app.zoom;
    let fallback = to_world(rect.center(), pan, zoom);

    draw_grid(&painter, rect, pan, zoom, &visuals);
    draw_net(app, &painter, fallback, pan, zoom, &visuals);

    if app.mode == EditMode::Connect {
        if let Some(from) = app.connect_from {
            if let Some(mouse) = response.hover_pos() {
                draw_connect_preview(app, &painter, from, to_world(mouse, pan, zoom), fallback, pan, zoom, &visuals);
            }
        }
    }

    if let (Some(start), Some(current)) = (app.marquee_start, app.marquee_current) {
        for node in nodes_in_rect(app, egui::Rect::from_two_pos(start, current)) {
            draw_selection_halo(
                &painter,
                node,
                to_screen(node_pos(app, node, fallback), pan, zoom),
                zoom,
                visuals.selection.bg_fill,
            );
        }
        draw_marquee(&painter, to_screen(start, pan, zoom), to_screen(current, pan, zoom), visuals.selection.bg_fill);
    }

    // Pan: middle-mouse drag and trackpad/wheel scroll. Ctrl+scroll zooms toward the cursor
    // instead. All gated on hovering the canvas so they don't fight other panels' scrolling.
    //
    // `i.smooth_scroll_delta` is NOT what fires during a ctrl/cmd+scroll gesture: egui's own
    // input handling (see `InputState::begin_pass`) detects the zoom modifier itself and routes
    // that scroll into `zoom_delta()` instead, leaving `smooth_scroll_delta` at zero. So the fix
    // isn't "check modifiers ourselves" (that races against something already zeroed) — it's to
    // read `zoom_delta()`, which is the same channel pinch-zoom gestures use.
    if response.hovered() {
        ui.input(|i| {
            if i.pointer.button_down(egui::PointerButton::Middle) {
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
            if let Some(p) = app.positions.get_mut(&node) {
                *p += response.drag_delta() / zoom;
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
            if i.key_pressed(egui::Key::Space) {
                toggle_simulate(app);
            }
        });
    }
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
                .corner_radius(14.0)
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
    window.show(ctx, add_contents).map(|inner| inner.response.rect)
}

fn destructive_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let danger = egui::Color32::from_rgb(224, 82, 82);
    ui.add(
        egui::Button::new((icons::icon("trash-2", 14.0).color(danger), egui::RichText::new(label).color(danger)))
            .corner_radius(6.0)
            .stroke(egui::Stroke::new(1.0, danger.gamma_multiply(0.4))),
    )
    .on_hover_text("Supr")
}

/// Inspector header: a round icon badge (not a boxed card, just an avatar) next to the
/// entity's name/kind, e.g. a place's filled-circle glyph beside "p1" / "Place".
fn entity_title(ui: &mut egui::Ui, icon_name: &'static str, name: &str, kind: &str) {
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
            ui.label(egui::RichText::new(name).strong().size(15.0));
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
                    .min_size(egui::vec2(22.0, 22.0)),
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

fn arc_in_editor(app: &mut PetriApp, ui: &mut egui::Ui, p: crate::model::PlaceId, t: crate::model::TransitionId) {
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
                .min_size(egui::vec2(30.0, 26.0));
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
        let _ = app.net.add_arc_place_to_transition(p, t, tag.to_kind(weight));
    }
}

fn arc_out_editor(app: &mut PetriApp, ui: &mut egui::Ui, t: crate::model::TransitionId, p: crate::model::PlaceId) {
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
                    entity_title(ui, "circle", app.net.place_label(p), "Place");
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
                    entity_title(ui, "rectangle-vertical", app.net.transition_label(t), "Transition");
                    ui.add_space(12.0);
                    ui.separator();
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
            ui.weak("Selección múltiple");
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(14.0);
            if destructive_button(ui, "Eliminar todos").clicked() {
                delete_selected(app);
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
                .min_size(egui::vec2(34.0, 30.0));
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
            .min_size(egui::vec2(34.0, 30.0));
        if ui.add(simulate_button).on_hover_text("Simular (Espacio)").clicked() {
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
            .min_size(egui::vec2(32.0, 28.0))
    };
    ui.vertical_centered(|ui| {
        ui.horizontal(|ui| {
            let can_back = !app.sim_history.is_empty();
            let can_forward = !app.sim_future.is_empty();
            let can_reset = app.sim_initial.is_some();

            if ui.add_enabled(can_back, ctrl_btn("skip-back")).on_hover_text("Paso atrás").clicked() {
                step_back(app);
            }
            if ui.add_enabled(can_reset, ctrl_btn("rotate-ccw")).on_hover_text("Reiniciar").clicked() {
                reset_sim(app);
            }
            if ui.add_enabled(can_forward, ctrl_btn("skip-forward")).on_hover_text("Paso adelante").clicked() {
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
                let btn = egui::Button::new((icons::icon("zap", 13.0), app.net.transition_label(t).to_string()))
                    .corner_radius(6.0);
                if ui.add_sized([ui.available_width(), 28.0], btn).clicked() {
                    fire_step(app, t);
                }
            }
        }
    });
}
