use std::sync::atomic::{AtomicU64, Ordering};

use eframe::egui;
use egui_graphs::{Graph, MetadataFrame, SettingsInteraction, SettingsNavigation, SettingsStyle};
use petgraph::stable_graph::{DefaultIx, NodeIndex, StableGraph};

use crate::analysis::{ExploreError, StateGraph, explore};
use crate::editor::{marking_chips, section_label};
use crate::icons;
use crate::model::fire::enabled_transitions;
use crate::model::{Marking, PetriNet};
use crate::theme;

/// A layout that does nothing — see `StaticGraphView` for why. Required by egui_graphs'
/// `LayoutState` bound (persistable, so it needs the usual derives) even though it's never
/// actually read or written to.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct NoLayoutState;
impl egui_graphs::LayoutState for NoLayoutState {}

#[derive(Default)]
struct NoLayout;
impl egui_graphs::Layout<NoLayoutState> for NoLayout {
  fn from_state(_state: NoLayoutState) -> impl egui_graphs::Layout<NoLayoutState> {
    Self
  }
  fn next<N, E, Ty, Ix, Dn, De>(
    &mut self,
    _g: &mut egui_graphs::Graph<N, E, Ty, Ix, Dn, De>,
    _ui: &egui::Ui,
  ) where
    N: Clone,
    E: Clone,
    Ty: petgraph::EdgeType,
    Ix: petgraph::stable_graph::IndexType,
    Dn: egui_graphs::DisplayNode<N, E, Ty, Ix>,
    De: egui_graphs::DisplayEdge<N, E, Ty, Ix, Dn>,
  {
  }
  fn state(&self) -> NoLayoutState {
    NoLayoutState
  }
}

/// Same as `egui_graphs::DefaultGraphView` except for the layout: egui_graphs' built-in
/// `Random` layout unconditionally overwrites every node's position the first time it runs
/// (see its doc comment claiming otherwise — the code doesn't match it), so it stomps on the
/// deterministic circular layout `explore()` already assigns via `set_location`. Swapping in
/// this no-op layout means OUR positions are the only ones that ever apply — no dependence on
/// when/whether the library's one-shot randomizer has "already triggered" for a given id, which
/// is exactly the fragile timing that caused the giant-overlapping-nodes bug in the first place.
type StaticGraphView<'a> = egui_graphs::GraphView<
  'a,
  (),
  (),
  petgraph::Directed,
  DefaultIx,
  egui_graphs::DefaultNodeShape,
  egui_graphs::DefaultEdgeShape,
  NoLayoutState,
  NoLayout,
>;

fn legend_dot(ui: &mut egui::Ui, color: egui::Color32, label: &str) {
  ui.horizontal(|ui| {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
    ui.weak(label);
  });
}

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

/// Monotonic counter, one new value per `explore()` call — see `graph_id` below.
static EXPLORE_GEN: AtomicU64 = AtomicU64::new(0);

const GRAPH_ZOOM_STEP: f32 = 1.25;
const GRAPH_ZOOM_MIN: f32 = 0.1;
const GRAPH_ZOOM_MAX: f32 = 8.0;
/// How long a zoom-button click takes to settle into place. Dragging to pan is never eased
/// through this — it snaps 1:1 with the pointer every frame, since a laggy drag feels broken.
const GRAPH_ZOOM_ANIM_SECS: f32 = 0.15;

pub struct ReachabilityState {
  state_graph: StateGraph,
  graph: Graph<(), ()>,
  node_indices: Vec<NodeIndex>,
  warning: Option<String>,
  /// egui_graphs keys its persisted layout/view state (pan, zoom, and — critically — whether
  /// its one-shot random layout has already "triggered") by this id. Without a fresh id per
  /// exploration, a second `explore()` call reuses the previous graph's "already laid out"
  /// flag even though the new nodes all start at the same default position, collapsing
  /// fit-to-screen's bounding box and zooming in absurdly (giant overlapping nodes).
  graph_id: String,
  /// Set once at construction, forcing a fit-to-screen the first time this graph is shown.
  refit_pending: bool,
  /// States with no enabled transitions — computed once at construction (`state_graph` never
  /// changes after that), reused every frame in `show()` to recolor the "current state" node
  /// without recomputing this each time.
  is_dead_end: Vec<bool>,
  /// `net.fingerprint()` as of the last explore — `show()` compares against the live net each
  /// frame and re-explores on a mismatch, so editing the net while this panel is open (adding a
  /// place, rewiring an arc, …) doesn't leave the graph showing a stale structure.
  fingerprint: u64,
  /// Screen-space rect the graph widget was allocated into last frame. egui_graphs' pan is a
  /// plain screen-space offset with no idea where "its" widget currently sits — it's set once
  /// by fit-to-screen and never re-derived from the rect afterwards. So *any* movement of that
  /// rect (the dock panel resizing, or just scrolling the tab body — egui_dock wraps every tab
  /// in a `ScrollArea`, and this graph is tall enough to scroll) leaves the stored pan pointing
  /// at where the widget used to be, and the graph visibly swims. Re-fitting whenever this rect
  /// actually moves keeps it pinned to its panel.
  last_rect: egui::Rect,
  /// Canvas-space point centered in the viewport, and the zoom level — set either by dragging
  /// (snaps 1:1, see `show()`'s drag handling) or by the toolbar's zoom buttons (eased, see
  /// `nudge_zoom`). `show()` re-derives egui_graphs' screen-space `pan` fresh from wherever
  /// these currently sit, every frame — never eases `pan` itself, because pan and zoom don't
  /// ease linearly together (their relationship — "keep this canvas point under the viewport
  /// center" — is non-linear), so animating them as two separate scalars drifted the anchor
  /// sideways mid-transition instead of holding it still.
  target_center: egui::Vec2,
  target_zoom: f32,
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
          StateGraph {
            nodes: vec![net.marking()],
            edges: Vec::new(),
          },
          Some(format!(
            "Espacio de estados no acotado: {names} crece sin límite."
          )),
        )
      }
      Err(ExploreError::TooManyStates) => (
        StateGraph {
          nodes: vec![net.marking()],
          edges: Vec::new(),
        },
        Some(
          "Acotado, pero el espacio de estados exacto es demasiado grande para explorarlo."
            .to_string(),
        ),
      ),
    };

    // Which states have at least one enabled transition — the rest are dead ends, worth
    // calling out in a different color since they're where the net gets visibly stuck.
    let has_outgoing: std::collections::HashSet<usize> =
      state_graph.edges.iter().map(|e| e.from).collect();
    let is_dead_end: Vec<bool> = (0..state_graph.nodes.len())
      .map(|i| !has_outgoing.contains(&i))
      .collect();

    let n = state_graph.nodes.len().max(1) as f32;
    // A fixed circular layout instead of trusting egui_graphs' built-in one-shot random
    // placement — deterministic, always has a sane non-degenerate bounding box (unlike random
    // points that can happen to land close together), and sidesteps that layout's fragile
    // "have I already run for this id" caching entirely. See `StaticGraphView`.
    let radius = 70.0 + 14.0 * n;
    let mut graph: Graph<(), ()> = Graph::new(StableGraph::default());
    let node_indices: Vec<NodeIndex> = state_graph
      .nodes
      .iter()
      .enumerate()
      .map(|(i, marking)| {
        let idx = graph.add_node_with_label((), format!("{i}: {}", marking_text(net, marking)));
        // Default node fill is egui_graphs' own `widgets.inactive.fg_stroke.color` — a
        // dim gray meant for subtle button icons, not a filled shape on a busy canvas. Color
        // by role instead, so the graph doubles as a quick-glance diagram: the initial state,
        // dead ends (no enabled transitions), and everything else get distinct colors.
        if let Some(node) = graph.node_mut(idx) {
          let color = if i == 0 {
            theme::accent()
          } else if is_dead_end[i] {
            theme::danger()
          } else {
            theme::text_weak()
          };
          node.set_color(color);
          let angle = i as f32 / n * std::f32::consts::TAU;
          node.set_location(egui::pos2(radius * angle.cos(), radius * angle.sin()));
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
      graph_id: format!(
        "reachability-{}",
        EXPLORE_GEN.fetch_add(1, Ordering::Relaxed)
      ),
      refit_pending: true,
      is_dead_end,
      fingerprint: net.fingerprint(),
      last_rect: egui::Rect::NOTHING,
      // Placeholders — the initial `refit_pending: true` above forces `show()`'s first frame
      // to run fit-to-screen and immediately adopt whatever it computes as the real target
      // (see the `force_fit` branch there), before either of these could otherwise matter.
      target_zoom: 1.0,
      target_center: egui::Vec2::ZERO,
    }
  }

  /// Ids `show()` drives `egui::Context::animate_value_with_time` through, one per animated
  /// scalar (zoom, center.x, center.y) — split out since that API animates a single `f32` at
  /// a time.
  fn anim_ids(&self) -> (egui::Id, egui::Id, egui::Id) {
    (
      egui::Id::new((self.graph_id.as_str(), "zoom")),
      egui::Id::new((self.graph_id.as_str(), "center_x")),
      egui::Id::new((self.graph_id.as_str(), "center_y")),
    )
  }

  /// Multiplies the *target* zoom by `factor`. `target_center` is untouched — since `show()`
  /// always re-derives `pan` from `(target_center, target_zoom)` so that `target_center` maps
  /// to the viewport's center, leaving it alone here is what keeps that same canvas point
  /// centered through the whole eased transition, with no separate pan compensation needed.
  fn nudge_zoom(&mut self, factor: f32) {
    self.target_zoom = (self.target_zoom * factor).clamp(GRAPH_ZOOM_MIN, GRAPH_ZOOM_MAX);
  }

  pub fn show(
    &mut self,
    ui: &mut egui::Ui,
    net: &PetriNet,
    route_modal: &mut Option<crate::route_modal::RouteModal>,
  ) {
    let fingerprint = net.fingerprint();
    if fingerprint != self.fingerprint {
      *self = Self::explore(net);
    }

    if let Some(warning) = &self.warning {
      ui.horizontal_wrapped(|ui| {
        ui.label(icons::icon("triangle-alert", 14.0).color(ui.visuals().warn_fg_color));
        ui.colored_label(ui.visuals().warn_fg_color, warning);
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
      ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.weak("Sin transiciones habilitadas desde el estado inicial.");
      });
    } else {
      // Recolor the node matching the net's live marking every frame, so the graph tracks the
      // simulation instead of only ever showing the snapshot from when it was first explored.
      let current_marking = net.marking();
      for (i, marking) in self.state_graph.nodes.iter().enumerate() {
        if let Some(node) = self.graph.node_mut(self.node_indices[i]) {
          let color = if *marking == current_marking {
            theme::success()
          } else if i == 0 {
            theme::accent()
          } else if self.is_dead_end[i] {
            theme::danger()
          } else {
            theme::text_weak()
          };
          node.set_color(color);
        }
      }

      ui.horizontal(|ui| {
        legend_dot(ui, theme::success(), "Actual");
        ui.add_space(10.0);
        legend_dot(ui, theme::accent(), "Inicial");
        ui.add_space(10.0);
        legend_dot(ui, theme::danger(), "Sin transiciones");
        ui.add_space(10.0);
        legend_dot(ui, theme::text_weak(), "Intermedio");
      });
      ui.add_space(6.0);

      let interactions = SettingsInteraction::new()
        .with_dragging_enabled(true)
        .with_node_selection_enabled(true);
      // Positions come entirely from our own fixed circular layout (see `explore()` /
      // `StaticGraphView`), so this fit only needs to run once, to frame that (always
      // well-behaved) layout in the viewport — not on some fragile "did the library's
      // one-shot randomizer already run" timing.
      let force_fit = std::mem::take(&mut self.refit_pending);
      // `zoom_and_pan_enabled` stays at its default `false`: this graph sits inside
      // egui_dock's per-tab `ScrollArea` (every tab body is wrapped in one), and drag/scroll
      // handed to the graph directly kept fighting that outer scroll area — dragging the
      // panel's own scrollbar, or scrolling the panel, while the pointer happened to be over
      // the graph. Simplest fix: the graph is only ever moved through the toolbar buttons
      // below, never by dragging or scrolling it directly, so there's nothing left to fight.
      let navigation = SettingsNavigation::new().with_fit_to_screen_enabled(force_fit);

      let style = SettingsStyle::new()
        .with_node_stroke_hook(|selected, dragged, _color, _stroke, style| {
          let base = style.visuals.text_color();
          egui::Stroke::new(
            if selected || dragged { 2.4 } else { 1.6 },
            if selected || dragged {
              crate::theme::accent()
            } else {
              base
            },
          )
        })
        .with_edge_stroke_hook(|selected, _order, _stroke, style| {
          egui::Stroke::new(if selected { 2.2 } else { 1.4 }, style.visuals.text_color())
        });

      egui::Frame::default()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
          // ponytail: GraphView always claims ui.available_size(); without a fixed area
          // here it and an auto-sized Window feed each other and balloon to fill the
          // screen. A fixed *width* used to be hardcoded to 600 here, which overflowed
          // the dock panel (usually narrower) and let drags meant for the dock's resize
          // splitter land on the graph instead. Using the dock's real available width
          // keeps the widget inside the panel it's actually drawn in; re-fit-on-rect-move
          // below (see `last_rect`'s doc comment) keeps it from drifting when that rect
          // moves without changing size (e.g. the tab body's own scroll area).
          let graph_area = egui::vec2(ui.available_width(), 340.0);
          let rect = egui::Rect::from_min_size(ui.cursor().min, graph_area);
          if (rect.min - self.last_rect.min).length_sq() > 0.25
            || (rect.size() - self.last_rect.size()).length_sq() > 0.25
          {
            self.last_rect = rect;
            self.refit_pending = true;
          }
          let (zoom_id, cx_id, cy_id) = self.anim_ids();
          if force_fit {
            // Let the library's own fit-to-screen run untouched this frame (below) — then
            // adopt whatever it computes as the new target and snap the animation state to
            // match, so the *next* button click eases from here instead of from a stale
            // pre-fit target (or animating the fit itself, which isn't what "fit to view"
            // should feel like).
          } else {
            // Ease zoom and center independently, then derive `pan` from wherever *both*
            // eased values currently sit — never ease `pan` itself. `target_center` maps to
            // `rect.center()` by construction, at every single frame of the transition, not
            // just at its start and end, which is what actually keeps the anchor visually
            // still instead of drifting sideways mid-zoom (see the struct doc comment).
            let zoom =
              ui.ctx()
                .animate_value_with_time(zoom_id, self.target_zoom, GRAPH_ZOOM_ANIM_SECS);
            let cx =
              ui.ctx()
                .animate_value_with_time(cx_id, self.target_center.x, GRAPH_ZOOM_ANIM_SECS);
            let cy =
              ui.ctx()
                .animate_value_with_time(cy_id, self.target_center.y, GRAPH_ZOOM_ANIM_SECS);
            let mut meta = MetadataFrame::new(Some(self.graph_id.clone())).load(ui);
            meta.zoom = zoom;
            meta.pan = rect.center().to_vec2() - egui::vec2(cx, cy) * zoom;
            meta.save(ui);
          }

          let view_response = ui
            .allocate_ui(graph_area, |ui| {
              let mut view = StaticGraphView::new(&mut self.graph)
                .with_interactions(&interactions)
                .with_navigations(&navigation)
                .with_styles(&style)
                .with_id(Some(self.graph_id.clone()));
              ui.add(&mut view)
            })
            .inner;

          if force_fit {
            let meta = MetadataFrame::new(Some(self.graph_id.clone())).load(ui);
            self.target_zoom = meta.zoom;
            self.target_center = (rect.center().to_vec2() - meta.pan) / meta.zoom;
            // Zero animation time snaps the eased value to match instantly instead of easing
            // into it (see `animate_value`'s handling of `animation_time == 0.0`).
            ui.ctx()
              .animate_value_with_time(zoom_id, self.target_zoom, 0.0);
            ui.ctx()
              .animate_value_with_time(cx_id, self.target_center.x, 0.0);
            ui.ctx()
              .animate_value_with_time(cy_id, self.target_center.y, 0.0);
          } else if (view_response.dragged_by(egui::PointerButton::Primary)
            || view_response.dragged_by(egui::PointerButton::Middle))
            // `dragging_enabled` (see `interactions` above) lets individual nodes be
            // repositioned by drag too — that's handled entirely inside `ui.add(&mut view)`
            // above and tracked on `self.graph`, so only treat this as "pan the canvas" when
            // it's *not* also mid-drag on a node, or moving a node would drag the whole view
            // out from under it at the same time.
            && self.graph.dragged_node().is_none()
          {
            let delta = view_response.drag_delta();
            if delta != egui::Vec2::ZERO {
              // Dragging snaps 1:1 with the pointer (zero animation time) rather than easing
              // — direct manipulation should never feel laggy, only button-triggered moves do.
              self.target_center -= delta / self.target_zoom;
              ui.ctx()
                .animate_value_with_time(cx_id, self.target_center.x, 0.0);
              ui.ctx()
                .animate_value_with_time(cy_id, self.target_center.y, 0.0);
            }
          }

          // A small fixed (non-draggable — `fixed_pos` implies `movable(false)`) toolbar
          // pinned to the graph's own bottom-right corner, recomputed every frame from `rect`
          // so it tracks the panel instead of drifting like the graph content used to.
          // `+`/`-` zoom in place (`nudge_zoom`, eased); the last button asks for the same
          // one-shot fit-to-screen the panel already does on open/re-explore. Panning itself
          // is drag-to-pan on the graph (see above), not a toolbar control.
          egui::Area::new(egui::Id::new("reachability-graph-toolbar"))
            .fixed_pos(rect.right_bottom() - egui::vec2(8.0, 8.0))
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
              egui::Frame::default()
                .fill(theme::surface_raised())
                .corner_radius(6.0)
                .stroke(egui::Stroke::new(1.0, theme::line_strong()))
                .inner_margin(egui::Margin::symmetric(3, 3))
                .show(ui, |ui| {
                  let btn = |ui: &mut egui::Ui, icon_name: &'static str| {
                    ui.add(
                      egui::Button::new(icons::icon(icon_name, 13.0))
                        .corner_radius(4.0)
                        .min_size(egui::vec2(22.0, 20.0)),
                    )
                  };
                  ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if btn(ui, "minus").clicked() {
                      self.nudge_zoom(1.0 / GRAPH_ZOOM_STEP);
                    }
                    if btn(ui, "plus").clicked() {
                      self.nudge_zoom(GRAPH_ZOOM_STEP);
                    }
                    if btn(ui, "maximize")
                      .on_hover_text("Ajustar a la vista")
                      .clicked()
                    {
                      self.refit_pending = true;
                    }
                  });
                });
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

    match selected_state {
      Some(state_idx) => {
        let marking = &self.state_graph.nodes[state_idx];
        section_label(ui, "Marcado");
        marking_chips(ui, net, marking);
        if state_idx != 0
          && let Some((states, transitions)) =
            crate::analysis::path_to(net, &self.state_graph.nodes[0], marking)
          && ui.button("Ver ruta").clicked()
        {
          *route_modal = Some(crate::route_modal::RouteModal::new(
            net,
            states,
            transitions,
          ));
        }
        ui.add_space(10.0);

        let enabled = enabled_transitions(net, marking);
        section_label(ui, "Habilitadas");
        ui.add_space(2.0);
        let mut any_enabled = false;
        for t in net.transition_ids().filter(|t| enabled.contains(t)) {
          any_enabled = true;
          ui.horizontal(|ui| {
            ui.label(icons::icon("zap", 13.0).color(ui.visuals().text_color()));
            ui.label(net.transition_label(t));
          });
        }
        if !any_enabled {
          ui.weak("(ninguna)");
        }
        ui.add_space(8.0);

        section_label(ui, "Deshabilitadas");
        ui.add_space(2.0);
        let mut any_disabled = false;
        for t in net.transition_ids().filter(|t| !enabled.contains(t)) {
          any_disabled = true;
          ui.horizontal(|ui| {
            ui.label(icons::icon("ban", 13.0).color(ui.visuals().weak_text_color()));
            ui.label(
              egui::RichText::new(net.transition_label(t)).color(ui.visuals().weak_text_color()),
            );
          });
        }
        if !any_disabled {
          ui.weak("(ninguna)");
        }
      }
      None => {
        ui.weak("Seleccioná un estado del grafo para ver su marking.");
      }
    }
  }
}
