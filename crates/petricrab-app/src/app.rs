use crate::model::{Marking, PetriNet, PlaceId, TransitionId};
use std::collections::{HashMap, HashSet};

use crate::editor;
use crate::icons;
use crate::properties_panel::PropertiesState;
use crate::reachability_panel::ReachabilityState;
use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum NodeId {
  Place(PlaceId),
  Transition(TransitionId),
}

#[derive(Clone, Debug, Default)]
pub enum Selection {
  #[default]
  None,
  /// One or more selected nodes (single click, or a drag/marquee selection).
  Nodes(HashSet<NodeId>),
  /// Input arc place -> transition.
  ArcIn(PlaceId, TransitionId),
  /// Output arc transition -> place.
  ArcOut(TransitionId, PlaceId),
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
}

pub struct PetriApp {
  pub net: PetriNet,
  /// Where this net was last saved to/loaded from; `None` for a never-saved "Nuevo" project.
  pub file_path: Option<std::path::PathBuf>,
  /// Most-recently opened/saved paths, newest first. Persisted via eframe's storage (see
  /// `PersistedState`/`App::save` below), not part of the document itself.
  pub recent: Vec<std::path::PathBuf>,
  pub positions: HashMap<NodeId, egui::Pos2>,
  pub mode: EditMode,
  pub connect_from: Option<NodeId>,
  pub dragging: Option<NodeId>,
  pub selection: Selection,
  pub reachability: Option<ReachabilityState>,
  pub properties: Option<PropertiesState>,
  pub next_place_n: usize,
  pub next_transition_n: usize,
  /// Screen-space offset of the (infinite) world origin: `screen = world * zoom + pan`.
  pub pan: egui::Vec2,
  pub zoom: f32,
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
  /// Marking captured when the simulate panel was opened; "Reset" returns to this.
  pub sim_initial: Option<Marking>,
  /// Undo/redo stacks of markings visited while stepping through the simulation.
  pub sim_history: Vec<Marking>,
  pub sim_future: Vec<Marking>,
  /// Toasts queued via [`PetriApp::notify`], drained into an `egui_toast::Toasts` and shown
  /// once per frame in `ui()`.
  pub toast_queue: Vec<egui_toast::Toast>,
}

impl PetriApp {
  pub fn new() -> Self {
    Self {
      net: PetriNet::new(),
      file_path: None,
      recent: Vec::new(),
      positions: HashMap::new(),
      mode: EditMode::Select,
      connect_from: None,
      dragging: None,
      selection: Selection::None,
      reachability: None,
      properties: None,
      next_place_n: 0,
      next_transition_n: 0,
      pan: egui::Vec2::ZERO,
      zoom: 1.0,
      marquee_start: None,
      marquee_current: None,
      show_grid: true,
      context_target: None,
      rotation: HashMap::new(),
      selection_focus: None,
      simulate_open: false,
      sim_initial: None,
      sim_history: Vec::new(),
      sim_future: Vec::new(),
      toast_queue: Vec::new(),
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
}

impl eframe::App for PetriApp {
  fn save(&mut self, storage: &mut dyn eframe::Storage) {
    eframe::set_value(
      storage,
      eframe::APP_KEY,
      &PersistedState {
        recent: self.recent.clone(),
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

    egui::Panel::right("inspector")
      .frame(
        egui::Frame::default()
          .fill(visuals.panel_fill)
          .inner_margin(egui::Margin::symmetric(16, 16)),
      )
      .show_separator_line(false)
      .default_size(260.0)
      .min_size(200.0)
      .show(ui, |ui| {
        let explore_button =
          egui::Button::new((icons::icon("workflow", 15.0), "Explorar espacio de estados"))
            .corner_radius(6.0);
        if ui
          .add_sized([ui.available_width(), 32.0], explore_button)
          .clicked()
        {
          self.reachability = Some(ReachabilityState::explore(&self.net, &visuals));
        }

        ui.add_space(6.0);
        let properties_button =
          egui::Button::new((icons::icon("shield-check", 15.0), "Propiedades del net"))
            .corner_radius(6.0);
        if ui
          .add_sized([ui.available_width(), 32.0], properties_button)
          .clicked()
        {
          self.properties = Some(PropertiesState::compute(&self.net));
        }

        ui.add_space(18.0);
        editor::selection_panel(self, ui);
      });

    if let Some(reachability) = &mut self.reachability {
      let mut open = true;
      let rect = editor::floating_window(
        &ctx,
        &visuals,
        editor::WindowSpec {
          id: "reachability-graph",
          icon: "workflow",
          title: "Grafo de alcanzabilidad",
          default_size: egui::vec2(660.0, 520.0),
          min_size: egui::vec2(380.0, 320.0),
          max_size: Some(egui::vec2(960.0, 780.0)),
          movable: true,
        },
        &mut open,
        |ui| reachability.show(ui, &self.net),
      );
      // See ReachabilityState::note_window_moved: this is how the graph notices the
      // window moved and re-fits, instead of trusting egui_graphs' own (buggy) pan
      // compensation across the move.
      if let Some(rect) = rect {
        reachability.note_window_moved(rect.left_top());
      }
      if !open {
        self.reachability = None;
      }
    }

    if let Some(properties) = &self.properties {
      let mut open = true;
      editor::floating_window(
        &ctx,
        &visuals,
        editor::WindowSpec {
          id: "net-properties",
          icon: "shield-check",
          title: "Propiedades del net",
          default_size: egui::vec2(340.0, 480.0),
          min_size: egui::vec2(260.0, 240.0),
          max_size: None,
          movable: true,
        },
        &mut open,
        |ui| {
          egui::ScrollArea::vertical().show(ui, |ui| {
            properties.show(ui, &self.net);
          });
        },
      );
      if !open {
        self.properties = None;
      }
    }

    egui::CentralPanel::default().show(ui, |ui| {
      editor::canvas(self, ui);
    });

    // Not `.anchor(...)`, that forces the area immovable. `default_pos` + `pivot` gives the
    // same starting position but leaves it draggable.
    egui::Area::new(egui::Id::new("toolbar"))
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
      });

    let mut toasts = egui_toast::Toasts::new()
      .anchor(egui::Align2::RIGHT_BOTTOM, egui::pos2(-12.0, -12.0))
      .direction(egui::Direction::BottomUp);
    for toast in self.toast_queue.drain(..) {
      toasts.add(toast);
    }
    toasts.show(ui);

    if self.simulate_open {
      egui::Area::new(egui::Id::new("simulate-popup"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -72.0))
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
  }
}
