use crate::model::{Marking, PetriNet, PlaceId, TransitionId};
use std::collections::{HashMap, HashSet};

use crate::editor;
use crate::icons;
use crate::properties_panel::PropertiesState;
use crate::reachability_panel::ReachabilityState;
use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    Select,
    AddPlace,
    AddTransition,
    Connect,
}

pub struct PetriApp {
    pub net: PetriNet,
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
    pub simulate_open: bool,
    /// Marking captured when the simulate panel was opened; "Reset" returns to this.
    pub sim_initial: Option<Marking>,
    /// Undo/redo stacks of markings visited while stepping through the simulation.
    pub sim_history: Vec<Marking>,
    pub sim_future: Vec<Marking>,
}

impl PetriApp {
    pub fn new() -> Self {
        Self {
            net: PetriNet::new(),
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
            simulate_open: false,
            sim_initial: None,
            sim_history: Vec::new(),
            sim_future: Vec::new(),
        }
    }
}

impl eframe::App for PetriApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let visuals = ui.visuals().clone();

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
                let explore_button = egui::Button::new((
                    icons::icon("workflow", 15.0),
                    "Explorar espacio de estados",
                ))
                .corner_radius(6.0);
                if ui
                    .add_sized([ui.available_width(), 32.0], explore_button)
                    .clicked()
                {
                    self.reachability = Some(ReachabilityState::explore(&self.net));
                }

                ui.add_space(6.0);
                let properties_button = egui::Button::new((
                    icons::icon("shield-check", 15.0),
                    "Propiedades del net",
                ))
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
            egui::Window::new("Reachability graph")
                .frame(
                    egui::Frame::default()
                        .fill(visuals.panel_fill)
                        .stroke(egui::Stroke::new(1.0, visuals.window_stroke.color))
                        .corner_radius(14.0)
                        .shadow(visuals.window_shadow)
                        .inner_margin(egui::Margin::symmetric(16, 14)),
                )
                .default_size([660.0, 520.0])
                .min_size([380.0, 320.0])
                .max_size([960.0, 780.0])
                .resizable(true)
                .collapsible(true)
                // ponytail: egui_graphs' internal pan-compensation doubles the graph's on-screen
                // shift whenever this window's top-left moves (its own bug, see graph_view.rs
                // handle_node_drag/ViewState.last_top_left). Resizing from the corner doesn't
                // move top-left, so keeping the window non-draggable sidesteps it without
                // patching the vendored crate. Upgrade path: revisit if a fixed version ships.
                .movable(false)
                .open(&mut open)
                .show(&ctx, |ui| {
                    reachability.show(ui, &self.net);
                });
            if !open {
                self.reachability = None;
            }
        }

        if let Some(properties) = &self.properties {
            let mut open = true;
            egui::Window::new("Propiedades del net")
                .frame(
                    egui::Frame::default()
                        .fill(visuals.panel_fill)
                        .stroke(egui::Stroke::new(1.0, visuals.window_stroke.color))
                        .corner_radius(14.0)
                        .shadow(visuals.window_shadow)
                        .inner_margin(egui::Margin::symmetric(16, 14)),
                )
                .default_size([340.0, 480.0])
                .min_size([260.0, 240.0])
                .resizable(true)
                .collapsible(true)
                .open(&mut open)
                .show(&ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        properties.show(ui, &self.net);
                    });
                });
            if !open {
                self.properties = None;
            }
        }

        egui::CentralPanel::default().show(ui, |ui| {
            editor::canvas(self, ui);
        });

        egui::Area::new(egui::Id::new("toolbar"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -18.0))
            .show(&ctx, |ui| {
                egui::Frame::default()
                    .fill(visuals.panel_fill)
                    .stroke(egui::Stroke::new(1.0, visuals.window_stroke.color))
                    .corner_radius(14.0)
                    .shadow(visuals.window_shadow)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        editor::toolbar(self, ui);
                    });
            });

        if self.simulate_open {
            egui::Area::new(egui::Id::new("simulate-popup"))
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -72.0))
                .show(&ctx, |ui| {
                    egui::Frame::default()
                        .fill(visuals.panel_fill)
                        .stroke(egui::Stroke::new(1.0, visuals.window_stroke.color))
                        .corner_radius(10.0)
                        .shadow(visuals.window_shadow)
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            editor::simulate_panel(self, ui);
                        });
                });
        }
    }
}
