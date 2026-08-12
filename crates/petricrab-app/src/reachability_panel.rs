use eframe::egui;
use egui_graphs::{DefaultGraphView, Graph, SettingsInteraction, SettingsNavigation, SettingsStyle};
use petgraph::stable_graph::{NodeIndex, StableGraph};

use crate::analysis::{ExploreError, StateGraph, explore};
use crate::editor::{card, section_label, token_chip};
use crate::icons;
use crate::model::fire::enabled_transitions;
use crate::model::{Marking, PetriNet};

pub(crate) fn marking_text(net: &PetriNet, marking: &Marking) -> String {
    let parts: Vec<String> = marking
        .iter()
        .filter(|&(_, &tokens)| tokens > 0)
        .map(|(&place, &tokens)| format!("{} {tokens}", net.place_label(place)))
        .collect();
    if parts.is_empty() {
        "(vacío)".to_string()
    } else {
        parts.join(", ")
    }
}

pub struct ReachabilityState {
    state_graph: StateGraph,
    graph: Graph<(), ()>,
    node_indices: Vec<NodeIndex>,
    warning: Option<String>,
    /// Screen top-left of the enclosing window, last frame — used only to notice it moved.
    last_top_left: Option<egui::Pos2>,
    /// Set for one frame after the window moves, forcing a re-fit instead of trusting
    /// egui_graphs' own pan bookkeeping (see `note_window_moved`).
    refit_pending: bool,
}

impl ReachabilityState {
    pub fn explore(net: &PetriNet, visuals: &egui::Visuals) -> Self {
        // Boundedness is checked (Karp-Miller, always terminates) before any BFS enumeration —
        // an unbounded net gets a precise "these places grow forever" instead of a truncation
        // warning after silently hitting an arbitrary state-count cap.
        let (state_graph, warning) = match explore(net, &net.marking()) {
            Ok(graph) => (graph, None),
            Err(ExploreError::Unbounded(places)) => {
                let names = places
                    .iter()
                    .map(|&p| net.place_label(p))
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    StateGraph { nodes: vec![net.marking()], edges: Vec::new() },
                    Some(format!("Espacio de estados no acotado: {names} crece sin límite.")),
                )
            }
            Err(ExploreError::TooManyStates) => (
                StateGraph { nodes: vec![net.marking()], edges: Vec::new() },
                Some(
                    "Acotado, pero el espacio de estados exacto es demasiado grande para explorarlo."
                        .to_string(),
                ),
            ),
        };

        let mut graph: Graph<(), ()> = Graph::new(StableGraph::default());
        let node_indices: Vec<NodeIndex> = state_graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, marking)| {
                let idx =
                    graph.add_node_with_label((), format!("{i}: {}", marking_text(net, marking)));
                // Default node fill is egui_graphs' own `widgets.inactive.fg_stroke.color` — a
                // dim gray meant for subtle button icons, not a filled shape on a busy canvas.
                // Use the app's own accent instead, so this graph reads as part of the same app
                // instead of a generic library demo dropped in unstyled.
                if let Some(node) = graph.node_mut(idx) {
                    node.set_color(visuals.selection.bg_fill);
                }
                idx
            })
            .collect();
        for edge in &state_graph.edges {
            graph.add_edge_with_label(
                node_indices[edge.from],
                node_indices[edge.to],
                (),
                net.transition_label(edge.via).to_string(),
            );
        }

        Self {
            state_graph,
            graph,
            node_indices,
            warning,
            last_top_left: None,
            refit_pending: true,
        }
    }

    /// Called once per frame with the enclosing window's current screen top-left. If it moved
    /// since last frame, requests a re-fit on the *next* `show()` — see the comment in `show()`
    /// on why we don't trust egui_graphs' own pan bookkeeping across a window move.
    pub fn note_window_moved(&mut self, top_left: egui::Pos2) {
        if self.last_top_left.is_some_and(|p| p != top_left) {
            self.refit_pending = true;
        }
        self.last_top_left = Some(top_left);
    }

    pub fn show(&mut self, ui: &mut egui::Ui, net: &PetriNet) {
        if let Some(warning) = &self.warning {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(icons::icon("triangle-alert", 14.0).color(ui.visuals().warn_fg_color));
                    ui.colored_label(ui.visuals().warn_fg_color, warning);
                });
            });
            ui.add_space(10.0);
        }

        ui.horizontal(|ui| {
            ui.label(icons::icon("workflow", 13.0));
            ui.weak(format!(
                "{} estados, {} transiciones",
                self.state_graph.nodes.len(),
                self.state_graph.edges.len()
            ));
        });
        ui.add_space(8.0);

        // A single-node graph has degenerate (zero-size) bounds; egui_graphs' fit-to-screen
        // then divides by ~0 and zooms in absurdly (one node fills the whole view). Below one
        // edge there's nothing worth panning/zooming into anyway, so skip the widget entirely.
        if self.state_graph.edges.is_empty() {
            card(ui, |ui| {
                ui.set_min_height(120.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.weak("Sin transiciones habilitadas desde el estado inicial.");
                });
            });
        } else {
            let interactions = SettingsInteraction::new()
                .with_dragging_enabled(true)
                .with_node_selection_enabled(true);
            // egui_graphs 0.31 double-counts its own pan compensation when the *container's*
            // screen position changes between frames (confirmed by reading graph_view.rs: it
            // both nudges `pan` by the top-left delta AND re-adds the new top-left at draw
            // time). Rather than patch around that internally, we detect the move ourselves
            // (`note_window_moved`, called from app.rs with the enclosing Window's rect) and
            // force exactly one real re-fit the frame after it happens — using the library's own
            // correct "fit to bounds" path instead of trusting its incremental pan math across a
            // move. `refit_pending` is consumed (reset to false) right after being read, so a
            // continuous drag re-fits every frame (looks like the graph rides along with the
            // window) and a stationary graph goes back to free pan/zoom immediately after.
            let force_fit = std::mem::take(&mut self.refit_pending);
            let navigation = SettingsNavigation::new().with_fit_to_screen_enabled(force_fit);

            let style = SettingsStyle::new()
                .with_node_stroke_hook(|selected, dragged, _color, _stroke, style| {
                    let base = style.visuals.text_color();
                    egui::Stroke::new(
                        if selected || dragged { 2.4 } else { 1.6 },
                        if selected || dragged { style.visuals.strong_text_color() } else { base },
                    )
                })
                .with_edge_stroke_hook(|selected, _order, _stroke, style| {
                    egui::Stroke::new(
                        if selected { 2.2 } else { 1.4 },
                        style.visuals.text_color(),
                    )
                });

            egui::Frame::default()
                .fill(ui.visuals().extreme_bg_color)
                .corner_radius(10.0)
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    // ponytail: GraphView always claims ui.available_size(); without a fixed area
                    // here it and an auto-sized Window feed each other and balloon to fill the
                    // screen. Upgrade path: track the window's actual size if resizing it becomes
                    // annoying.
                    let graph_area = egui::vec2(600.0, 340.0);
                    ui.allocate_ui(graph_area, |ui| {
                        let mut view = DefaultGraphView::new(&mut self.graph)
                            .with_interactions(&interactions)
                            .with_navigations(&navigation)
                            .with_styles(&style);
                        ui.add(&mut view);
                    });
                });
        }
        ui.add_space(10.0);

        section_label(ui, "Información del estado");
        ui.add_space(6.0);
        let selected_state = self
            .graph
            .selected_nodes()
            .first()
            .and_then(|idx| self.node_indices.iter().position(|node| node == idx));

        card(ui, |ui| match selected_state {
            Some(state_idx) => {
                let marking = &self.state_graph.nodes[state_idx];
                ui.weak("Marking");
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
                ui.add_space(8.0);
                ui.weak("Transiciones");
                let enabled = enabled_transitions(net, marking);
                for t in net.transition_ids() {
                    let is_enabled = enabled.contains(&t);
                    ui.horizontal(|ui| {
                        let (icon_name, color) = if is_enabled {
                            ("zap", ui.visuals().text_color())
                        } else {
                            ("ban", ui.visuals().weak_text_color())
                        };
                        ui.label(icons::icon(icon_name, 13.0).color(color));
                        ui.label(
                            egui::RichText::new(net.transition_label(t))
                                .color(if is_enabled { ui.visuals().text_color() } else { ui.visuals().weak_text_color() }),
                        );
                    });
                }
            }
            None => {
                ui.weak("Seleccioná un estado del grafo para ver su marking.");
            }
        });
    }
}
