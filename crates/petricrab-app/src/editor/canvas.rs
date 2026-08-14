use std::collections::HashSet;

use eframe::egui;

use crate::app::{ContextTarget, EditMode, NodeId, PetriApp, Selection};
use crate::icons;
use crate::model::{ArcKind, fire};
use crate::theme;

use super::clipboard::{copy_selection, paste_clipboard};
use super::draw::{
  draw_connect_preview, draw_grid, draw_marquee, draw_net, draw_notes, draw_selection_halo,
  note_edit_overlay,
};
use super::geometry::{
  compatible, hit_test, hit_test_arc, node_pos, nodes_in_rect, note_hit_test, note_resize_hit_test,
  snap_to_grid, to_screen, to_world,
};
use super::history::{checkpoint, redo, undo};
use super::panels::{ArcKindTag, arrow_row, rotation_control};

const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 3.0;
const NOTE_MIN_SIZE: egui::Vec2 = egui::vec2(90.0, 54.0);

/// Fires `t` and records the pre-fire marking so the simulator can step back through it. Used
/// by both the simulate panel's buttons and the canvas token-game, so stepping back undoes
/// whichever one you used to fire.
pub(crate) fn fire_step(app: &mut PetriApp, t: crate::model::TransitionId) {
  app.sim_history.push(app.net.marking());
  app.sim_future.clear();
  let _ = fire::fire(&mut app.net, t);
}

pub(crate) fn step_back(app: &mut PetriApp) {
  if let Some(prev) = app.sim_history.pop() {
    app.sim_future.push(app.net.marking());
    app.net.set_marking(&prev);
  }
}

pub(crate) fn step_forward(app: &mut PetriApp) {
  if let Some(next) = app.sim_future.pop() {
    app.sim_history.push(app.net.marking());
    app.net.set_marking(&next);
  }
}

pub(crate) fn reset_sim(app: &mut PetriApp) {
  if let Some(initial) = app.sim_initial.clone() {
    app.net.set_marking(&initial);
    app.sim_history.clear();
    app.sim_future.clear();
  }
}

/// Opening the simulate panel snapshots the current marking as the "Reset" target and starts a
/// fresh undo/redo history for this session.
pub(crate) fn toggle_simulate(app: &mut PetriApp) {
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

pub(crate) fn delete_selected(app: &mut PetriApp) {
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

pub(crate) fn apply_mode(app: &mut PetriApp, mode: EditMode) {
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

    // `command` is Ctrl on Windows/Linux, Cmd on Mac. These run unconditionally — not gated on
    // `no_focus` below — because that gate exists only to stop the bare-letter mode shortcuts
    // (C, V, P...) from hijacking a label rename in progress; a Ctrl-held combo never collides
    // with typing, so requiring "nothing focused" here just meant a stray focused widget (e.g.
    // a rename field that never lost focus) silently swallowed undo/copy/paste.
    //
    // Copy/paste specifically can't be read via `key_pressed(Key::C/V)` at all: egui-winit
    // intercepts Ctrl+C/Ctrl+V at the OS-event level and turns them into `Event::Copy` /
    // `Event::Paste(_)` instead of a plain key event (see `is_copy_command`/`is_paste_command`
    // in egui-winit), so those are the events to look for here.
    let (do_redo, do_undo, do_copy, do_paste) = ui.input(|i| {
      let redo = i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z);
      let undo = !redo && i.modifiers.command && i.key_pressed(egui::Key::Z);
      let copy = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
      let paste = i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)));
      (redo, undo, copy, paste)
    });
    if do_redo {
      redo(app);
    }
    if do_undo {
      undo(app);
    }
    if do_copy {
      copy_selection(app, ui.ctx());
    }
    if do_paste {
      paste_clipboard(app);
    }

    let no_focus = ui.memory(|m| m.focused().is_none());
    if no_focus {
      let delete_pressed =
        ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
      if delete_pressed && !matches!(app.selection, Selection::None) {
        delete_selected(app);
      }

      ui.input(|i| {
        // Excludes `command` so Ctrl+C/Ctrl+V (handled above) don't also flip the mode.
        if !i.modifiers.command && i.key_pressed(egui::Key::C) {
          apply_mode(app, EditMode::Connect);
        }
        if !i.modifiers.command && i.key_pressed(egui::Key::V) {
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

pub(crate) fn reset_view(app: &mut PetriApp) {
  app.pan = egui::Vec2::ZERO;
  app.zoom = 1.0;
}

/// Pans (without changing zoom) so `node`'s world position lands at the center of the canvas.
pub fn center_on_node(app: &mut PetriApp, node: NodeId) {
  let pos = node_pos(app, node, egui::Pos2::ZERO);
  app.pan = app.canvas_rect.center().to_vec2() - pos.to_vec2() * app.zoom;
}

/// Sets up a flat, cohesive look for a dropdown/context menu's contents: roomier row padding,
/// and a muted hover fill. `menu_item`/`danger_menu_item`/raw `Button`s in these scopes opt out
/// of the *idle* frame themselves via `.frame_when_inactive(false)`, so only hover/press ever
/// paint a background — dialed down here so it reads as a highlight, not a loud box.
pub(crate) fn begin_flat_menu(ui: &mut egui::Ui) {
  ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
  let muted = theme::surface_hover().gamma_multiply(0.5);
  ui.visuals_mut().widgets.hovered.weak_bg_fill = muted;
  ui.visuals_mut().widgets.hovered.bg_fill = muted;
}

pub(crate) fn menu_item(ui: &mut egui::Ui, icon_name: &'static str, label: &str) -> egui::Response {
  ui.add(
    egui::Button::new((icons::icon(icon_name, 13.0), label))
      .corner_radius(6.0)
      .frame_when_inactive(false),
  )
}

/// A menu row that toggles something on/off — a button, not a checkbox: same icon+label shape
/// as `menu_item`, but tinted with the app's selected/active style when `checked` (matching how
/// the toolbar's mode buttons show which one is active) and a check glyph up front.
pub(crate) fn toggle_menu_item(ui: &mut egui::Ui, checked: bool, label: &str) -> egui::Response {
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
