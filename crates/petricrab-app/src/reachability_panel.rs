use eframe::egui;
use egui_graphs::{DefaultGraphView, Graph, SettingsInteraction, SettingsNavigation};
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
}

impl ReachabilityState {
    pub fn explore(net: &PetriNet) -> Self {
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
                graph.add_node_with_label((), format!("{i}: {}", marking_text(net, marking)))
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
        }
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
            // ponytail: fit_to_screen_enabled(true) (the default) re-fits every single frame,
            // fighting any manual pan/zoom the moment you try it. Fit once on open instead.
            let navigation = SettingsNavigation::new().with_fit_to_screen_enabled(false);

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
                            .with_navigations(&navigation);
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
