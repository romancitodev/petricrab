use crate::model::{Marking, PetriNet, PlaceId, TransitionId};
use std::collections::{HashMap, HashSet};

use crate::editor;
use crate::properties_panel::PropertiesState;
use crate::reachability_panel::ReachabilityState;
use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum NodeId {
  Place(PlaceId),
  Transition(TransitionId),
}

slotmap::new_key_type! {
  /// A free-form text annotation on the canvas (legends, comments) — not part of the Petri
  /// net itself, so it lives here rather than in `model::PetriNet`.
  pub struct NoteId;
}

#[derive(Clone)]
pub struct NoteData {
  /// Top-left corner, world space (not centered — makes corner-resize math trivial: the
  /// dragged corner moves, this one stays put).
  pub pos: egui::Pos2,
  pub size: egui::Vec2,
  pub text: String,
  /// `None` falls back to the theme's default note fill.
  pub color: Option<egui::Color32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Selection {
  #[default]
  None,
  /// One or more selected nodes (single click, or a drag/marquee selection).
  Nodes(HashSet<NodeId>),
  /// Input arc place -> transition.
  ArcIn(PlaceId, TransitionId),
  /// Output arc transition -> place.
  ArcOut(TransitionId, PlaceId),
  Note(NoteId),
}

/// What a right-click landed on, captured once when the context menu opens (see
/// `editor::canvas`) so the menu's contents stay stable while the pointer wanders off the canvas
/// and onto the menu itself.
#[derive(Clone, Copy, Debug)]
pub enum ContextTarget {
  Node(NodeId),
  ArcIn(PlaceId, TransitionId),
  ArcOut(TransitionId, PlaceId),
  /// Empty canvas, at this world-space position.
  Empty(egui::Pos2),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
  Select,
  AddPlace,
  AddTransition,
  Connect,
  AddNote,
}

pub struct PetriApp {
  pub net: PetriNet,
  /// Where this net was last saved to/loaded from; `None` for a never-saved "Nuevo" project.
  pub file_path: Option<std::path::PathBuf>,
  /// Most-recently opened/saved paths, newest first. Persisted via eframe's storage (see
  /// `PersistedState`/`App::save` below), not part of the document itself.
  pub recent: Vec<std::path::PathBuf>,
  pub positions: HashMap<NodeId, egui::Pos2>,
  /// Per-place custom fill color, set from the place's context menu. Missing entry = the
  /// theme's default place fill (same sparse-map convention as `rotation` below).
  pub colors: HashMap<PlaceId, egui::Color32>,
  /// Free-form text annotations on the canvas — not part of the net, deliberately kept out of
  /// `positions`/`NodeId`/`Selection::Nodes` (no arcs, no multi-select group-drag) since none
  /// of that machinery applies to them.
  pub notes: slotmap::SlotMap<NoteId, NoteData>,
  pub dragging_note: Option<NoteId>,
  /// Set while the corner resize handle of a note is being dragged (see `editor::canvas`).
  pub resizing_note: Option<NoteId>,
  /// The note whose text is live-editable right now (see `editor::note_edit_overlay`). Distinct
  /// from `Selection::Note`: a single click only selects/focuses a note, a second click on an
  /// already-selected note is what opens it for typing.
  pub editing_note: Option<NoteId>,
  /// Snapshot of "was this note already selected" taken the instant a press on it starts (see
  /// `editor::canvas`'s `drag_started` handling), since by the time the click resolves,
  /// `selection` has already been eagerly set to that same note either way. `handle_click` reads
  /// this to tell a fresh click (focus only) from a click on an already-selected note (edit).
  pub reselecting_note: bool,
  pub mode: EditMode,
  pub connect_from: Option<NodeId>,
  pub dragging: Option<NodeId>,
  pub selection: Selection,
  pub reachability: Option<ReachabilityState>,
  pub properties: Option<PropertiesState>,
  /// Which analysis panels are currently docked to the right of the canvas — starts empty
  /// (no panel takes up space until the user opens one via the "Ver" menu or an inspector
  /// button). The tabs' actual data still lives in `reachability`/`properties` above; this
  /// only tracks placement.
  pub dock: egui_dock::DockState<crate::dock::DockTab>,
  pub next_place_n: usize,
  pub next_transition_n: usize,
  /// Screen-space offset of the (infinite) world origin: `screen = world * zoom + pan`.
  pub pan: egui::Vec2,
  pub zoom: f32,
  /// Screen-space rect the canvas painted into last frame — refreshed every frame in
  /// `editor::canvas`. Used to center the view on a node (e.g. from the outline panel)
  /// without threading the canvas response back out of `canvas()` itself.
  pub canvas_rect: egui::Rect,
  /// World-space marquee (rubber-band select) rectangle corners, while dragging.
  pub marquee_start: Option<egui::Pos2>,
  pub marquee_current: Option<egui::Pos2>,
  pub show_grid: bool,
  /// Set on right-click, read by the context menu contents while it stays open.
  pub context_target: Option<ContextTarget>,
  /// A transition's orientation, in degrees clockwise from the default vertical bar. Missing
  /// entry = 0°.
  pub rotation: HashMap<TransitionId, f32>,
  /// When several nodes are selected, which one (if any) the inspector's per-item editor is
  /// currently expanded for.
  pub selection_focus: Option<NodeId>,
  pub simulate_open: bool,
  /// Mirrors `theme::is_light()` so the "Ver" menu checkbox has something to bind to; the
  /// actual active palette lives in `theme`, this is just the UI-facing copy.
  pub light_mode: bool,
  /// Marking captured when the simulate panel was opened; "Reset" returns to this.
  pub sim_initial: Option<Marking>,
  /// Undo/redo stacks of markings visited while stepping through the simulation.
  pub sim_history: Vec<Marking>,
  pub sim_future: Vec<Marking>,
  /// Toasts queued via [`PetriApp::notify`], drained into an `egui_toast::Toasts` and shown
  /// once per frame in `ui()`.
  pub toast_queue: Vec<egui_toast::Toast>,
  /// Set from the "Ver ruta" button on a deadlock; drawn as a blocking `egui::Modal` in `ui()`
  /// (unlike `simulate_open`'s free-form popup, this one locks input to its own step-through).
  pub route_modal: Option<crate::route_modal::RouteModal>,
  /// Screen-space rect the floating mode toolbar painted into last frame — refreshed every
  /// frame in `ui()` below, same idea as `canvas_rect`. Lets the tutorial spotlight point at it
  /// without the toolbar needing to know the tutorial exists.
  pub toolbar_rect: egui::Rect,
  /// Screen-space rect the "Ver" menu button painted into last frame ("Propiedades del net"
  /// lives there), same purpose as `toolbar_rect` — set in `editor::menu_bar`, since that's
  /// where the button itself lives.
  pub menu_ver_rect: egui::Rect,
  /// Screen-space rect the right-hand dock panel (Selección/Propiedades/etc.) painted into last
  /// frame, when one is open — only meaningful while `dock.main_surface().num_tabs() > 0`.
  pub dock_panel_rect: egui::Rect,
  /// Guided first-run walkthrough; `None` once dismissed (finished/skipped/Esc) or never shown.
  pub tutorial: Option<crate::tutorial::TutorialState>,
  /// Persisted so the walkthrough only auto-opens once, ever, per install.
  pub tutorial_seen: bool,
  /// Undo/redo stacks (Ctrl+Z / Ctrl+Shift+Z) of full-state snapshots — see `editor::Snapshot`
  /// and `editor::checkpoint`.
  pub undo_stack: Vec<editor::Snapshot>,
  pub redo_stack: Vec<editor::Snapshot>,
  /// Last Ctrl+C'd selection, ready for Ctrl+V — see `editor::Clipboard`.
  pub clipboard: Option<editor::Clipboard>,
  /// World-space spacing `Align::Auto` uses when it distributes a selection along its dominant
  /// axis. User-adjustable from the "Alinear" section of the multi-select inspector.
  pub align_gap: f32,
}

impl PetriApp {
  pub fn new() -> Self {
    Self {
      net: PetriNet::new(),
      file_path: None,
      recent: Vec::new(),
      positions: HashMap::new(),
      colors: HashMap::new(),
      notes: slotmap::SlotMap::default(),
      dragging_note: None,
      resizing_note: None,
      editing_note: None,
      reselecting_note: false,
      mode: EditMode::Select,
      connect_from: None,
      dragging: None,
      selection: Selection::None,
      reachability: None,
      properties: None,
      dock: egui_dock::DockState::new(Vec::new()),
      next_place_n: 0,
      next_transition_n: 0,
      pan: egui::Vec2::ZERO,
      zoom: 1.0,
      canvas_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
      marquee_start: None,
      marquee_current: None,
      show_grid: true,
      context_target: None,
      rotation: HashMap::new(),
      selection_focus: None,
      simulate_open: false,
      light_mode: false,
      sim_initial: None,
      sim_history: Vec::new(),
      sim_future: Vec::new(),
      toast_queue: Vec::new(),
      route_modal: None,
      toolbar_rect: egui::Rect::NOTHING,
      menu_ver_rect: egui::Rect::NOTHING,
      dock_panel_rect: egui::Rect::NOTHING,
      tutorial: None,
      tutorial_seen: false,
      undo_stack: Vec::new(),
      redo_stack: Vec::new(),
      clipboard: None,
      align_gap: 96.0,
    }
  }

  /// Queues a toast notification, shown for a few seconds on the next frame.
  pub fn notify(&mut self, kind: egui_toast::ToastKind, text: impl Into<String>) {
    self.toast_queue.push(
      egui_toast::Toast::new()
        .kind(kind)
        .text(text.into())
        .options(
          egui_toast::ToastOptions::default()
            .duration_in_seconds(4.0)
            .show_progress(true)
            .show_icon(true),
        ),
    );
  }
}

/// The only slice of `PetriApp` that survives a restart — the document itself (net, positions,
/// view) is not persisted, just the recent-files list.
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct PersistedState {
  pub recent: Vec<std::path::PathBuf>,
  pub light_mode: bool,
  /// `#[serde(default)]`: an eframe storage file saved before this field existed is missing
  /// the key entirely — without a default, deserializing it fails outright and silently wipes
  /// `recent`/`light_mode` too (see `main.rs`'s `.unwrap_or_default()` fallback).
  #[serde(default)]
  pub tutorial_seen: bool,
}

impl eframe::App for PetriApp {
  fn save(&mut self, storage: &mut dyn eframe::Storage) {
    eframe::set_value(
      storage,
      eframe::APP_KEY,
      &PersistedState {
        recent: self.recent.clone(),
        light_mode: self.light_mode,
        tutorial_seen: self.tutorial_seen,
      },
    );
  }

  fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
    let ctx = ui.ctx().clone();
    let visuals = ui.visuals().clone();

    egui::Panel::top("menu_bar")
      .frame(
        egui::Frame::default()
          .fill(visuals.panel_fill)
          .stroke(egui::Stroke::new(1.0, visuals.window_stroke.color))
          .inner_margin(egui::Margin::symmetric(12, 4)),
      )
      .show_separator_line(false)
      .show(ui, |ui| {
        editor::menu_bar(self, ui, &ctx);
      });

    // The selected-element editor lives in the dock as its own tab now (see `DockTab::Selection`)
    // instead of a separate always-there-when-selected side panel — one dock, one place to look.
    crate::dock::sync_selection_tab(self);

    // `Tree::is_empty()` counts *nodes*, not tabs — a freshly-built empty dock still has one
    // (tab-less) leaf node, so that check never actually hides the panel. `num_tabs()` is the
    // one that means what we want: nothing open, no space taken.
    if self.dock.main_surface().num_tabs() > 0 {
      self.dock_panel_rect = egui::Panel::right("dock_panels")
        .frame(egui::Frame::default().fill(visuals.panel_fill))
        .default_size(340.0)
        .min_size(260.0)
        .show(ui, |ui| {
          let mut dock = std::mem::replace(&mut self.dock, egui_dock::DockState::new(Vec::new()));
          egui_dock::DockArea::new(&mut dock)
            .style(crate::dock::style(ui.style()))
            .show_inside(ui, &mut crate::dock::DockTabViewer { app: self });
          self.dock = dock;
        })
        .response
        .rect;
    }

    egui::Panel::bottom("status_bar")
      .frame(
        egui::Frame::default()
          .fill(visuals.panel_fill)
          .stroke(egui::Stroke::new(1.0, visuals.window_stroke.color))
          .inner_margin(egui::Margin::symmetric(12, 4)),
      )
      .show_separator_line(true)
      .exact_size(24.0)
      .show(ui, |ui| {
        ui.horizontal(|ui| {
          let total_tokens: u32 = self.net.place_ids().map(|p| self.net.tokens(p)).sum();
          ui.weak(format!("{} places", self.net.place_ids().count()));
          ui.separator();
          ui.weak(format!("{} transitions", self.net.transition_ids().count()));
          ui.separator();
          ui.weak(format!("{total_tokens} tokens"));
          ui.separator();
          ui.weak(format!("{:.0}%", self.zoom * 100.0));
          ui.separator();
          let selection_text = match &self.selection {
            Selection::None => "Nada seleccionado".to_string(),
            Selection::Nodes(nodes) if nodes.len() == 1 => match nodes.iter().next().unwrap() {
              NodeId::Place(p) if self.net.place_ids().any(|id| id == *p) => {
                self.net.place_label(*p).to_string()
              }
              NodeId::Transition(t) if self.net.transition_ids().any(|id| id == *t) => {
                self.net.transition_label(*t).to_string()
              }
              _ => "Nada seleccionado".to_string(),
            },
            Selection::Nodes(nodes) => format!("{} elementos", nodes.len()),
            Selection::ArcIn(..) => "Arco de entrada".to_string(),
            Selection::ArcOut(..) => "Arco de salida".to_string(),
            Selection::Note(_) => "Nota".to_string(),
          };
          ui.weak(selection_text);
        });
      });

    // Default `CentralPanel` frame has an 8px inner margin — enough to read as an accidental
    // crop around the canvas. Zero it out so the canvas paints edge-to-edge.
    egui::CentralPanel::default()
      .frame(
        egui::Frame::default()
          .fill(visuals.panel_fill)
          .inner_margin(0),
      )
      .show(ui, |ui| {
        editor::canvas(self, ui);
      });

    // Not `.anchor(...)`, that forces the area immovable. `default_pos` + `pivot` gives the
    // same starting position but leaves it draggable.
    self.toolbar_rect = egui::Area::new(egui::Id::new("toolbar"))
      .default_pos(egui::pos2(
        ctx.content_rect().center().x,
        ctx.content_rect().bottom() - 18.0,
      ))
      .pivot(egui::Align2::CENTER_BOTTOM)
      .movable(true)
      .constrain(true)
      .show(&ctx, |ui| {
        egui::Frame::default()
          .fill(visuals.panel_fill)
          .stroke(egui::Stroke::new(1.0, visuals.window_stroke.color))
          .corner_radius(crate::theme::RADIUS_LG)
          .shadow(visuals.window_shadow)
          .inner_margin(egui::Margin::symmetric(8, 6))
          .show(ui, |ui| {
            editor::toolbar(self, ui);
          });
      })
      .response
      .rect;

    let mut toasts = egui_toast::Toasts::new()
      .anchor(egui::Align2::RIGHT_BOTTOM, egui::pos2(-12.0, -12.0))
      .direction(egui::Direction::BottomUp);
    for toast in self.toast_queue.drain(..) {
      toasts.add(toast);
    }
    toasts.show(ui);

    if self.simulate_open {
      // `default_pos` + `pivot` (not `.anchor(...)`, see the toolbar above) starts the panel
      // over the toolbar but leaves it draggable, so it can be moved out of the way of whatever
      // part of the net is being simulated.
      egui::Area::new(egui::Id::new("simulate-popup"))
        .default_pos(egui::pos2(
          ctx.content_rect().center().x,
          ctx.content_rect().bottom() - 72.0,
        ))
        .pivot(egui::Align2::CENTER_BOTTOM)
        .movable(true)
        .constrain(true)
        .show(&ctx, |ui| {
          egui::Frame::default()
            .fill(visuals.panel_fill)
            .stroke(egui::Stroke::new(1.0, visuals.window_stroke.color))
            .corner_radius(crate::theme::RADIUS_LG)
            .shadow(visuals.window_shadow)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
              editor::simulate_panel(self, ui);
            });
        });
    }

    // Taken out (not just `&mut self.route_modal`) so `show` can take `&mut self` itself — it
    // needs the whole app (net, pan, canvas_rect) to drive the canvas and follow the camera.
    if let Some(mut route_modal) = self.route_modal.take() {
      if route_modal.show(&ctx, self) {
        self.route_modal = Some(route_modal);
      } else {
        route_modal.close(&mut self.net);
      }
    }

    if let Some(mut tutorial) = self.tutorial.take() {
      if crate::tutorial::show(&mut tutorial, &ctx, self) {
        self.tutorial = Some(tutorial);
      } else {
        self.tutorial_seen = true;
      }
    }
  }
}
