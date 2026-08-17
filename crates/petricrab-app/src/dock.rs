//! Tab identity + rendering glue for the dockable analysis panels (reachability graph,
//! properties, outline). The tabs' actual data keeps living on `PetriApp` as it always has
//! (`reachability`/`properties` are still `Option<State>`, `None` = not computed) — `DockTab`
//! only tracks which of them are currently placed in the dock, not their content.
use eframe::egui;

use crate::app::PetriApp;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DockTab {
  Reachability,
  Properties,
  Outline,
  Selection,
  Dsl,
  Help,
}

/// A dock chrome that reads as part of this app's own editor UI instead of a generic library
/// widget dropped in unstyled — same rounded-top-corner tab shape and corner radius the rest
/// of the app already uses for buttons/menus (6px), active tab raised a shade above the bar so
/// it's unambiguous which one is focused, flat (unrounded) content pane underneath, no
/// drop-shadow border. `Style::from_egui` derives sane state colors (hover/focus/kb-focus) as
/// a base; only the handful of fields below are overridden.
pub fn style(egui_style: &egui::Style) -> egui_dock::Style {
  let tab_corners = egui::CornerRadius {
    nw: 6,
    ne: 6,
    sw: 0,
    se: 0,
  };
  let mut style = egui_dock::Style::from_egui(egui_style);
  style.tab_bar.height = 32.0;
  style.tab_bar.corner_radius = tab_corners;
  style.tab_bar.bg_fill = crate::theme::surface();
  style.tab_bar.inner_margin = egui::Margin::symmetric(4, 0);
  style.tab.spacing = 2.0;
  style.tab.active.corner_radius = tab_corners;
  style.tab.active.bg_fill = crate::theme::surface_raised();
  style.tab.active.text_color = crate::theme::text_strong();
  style.tab.inactive.corner_radius = tab_corners;
  style.tab.inactive.bg_fill = crate::theme::surface();
  style.tab.inactive.text_color = crate::theme::text_weak();
  style.tab.focused.corner_radius = tab_corners;
  style.tab.hovered.corner_radius = tab_corners;
  style.tab.hovered.bg_fill = crate::theme::surface_hover();
  style.tab.tab_body.corner_radius = egui::CornerRadius::ZERO;
  style.tab.tab_body.stroke = egui::Stroke::NONE;
  style.tab.tab_body.bg_fill = crate::theme::surface();
  style.tab.hline_below_active_tab_name = false;
  style.main_surface_border_stroke = egui::Stroke::NONE;
  style.dock_area_padding = None;
  style
}

pub struct DockTabViewer<'a> {
  pub app: &'a mut PetriApp,
}

impl egui_dock::TabViewer for DockTabViewer<'_> {
  type Tab = DockTab;

  fn title(&mut self, tab: &mut DockTab) -> egui::WidgetText {
    match tab {
      DockTab::Reachability => "Grafo de alcanzabilidad".into(),
      DockTab::Properties => "Propiedades del net".into(),
      DockTab::Outline => "Estructura".into(),
      DockTab::Selection => "Selección".into(),
      DockTab::Dsl => "DSL".into(),
      DockTab::Help => "Ayuda".into(),
    }
  }

  fn ui(&mut self, ui: &mut egui::Ui, tab: &mut DockTab) {
    match tab {
      DockTab::Reachability => {
        let net = &self.app.net;
        let route_modal = &mut self.app.route_modal;
        if let Some(reachability) = &mut self.app.reachability {
          reachability.show(ui, net, route_modal);
        }
      }
      DockTab::Properties => {
        let net = &self.app.net;
        let route_modal = &mut self.app.route_modal;
        if let Some(properties) = &mut self.app.properties {
          egui::ScrollArea::vertical().show(ui, |ui| properties.show(ui, net, route_modal));
        }
      }
      DockTab::Outline => crate::editor::outline_panel(self.app, ui),
      DockTab::Selection => {
        egui::ScrollArea::vertical().show(ui, |ui| crate::editor::selection_panel(self.app, ui));
      }
      DockTab::Dsl => {
        egui::ScrollArea::vertical().show(ui, |ui| crate::dsl_panel::show(self.app, ui));
      }
      DockTab::Help => {
        egui::ScrollArea::vertical().show(ui, crate::help_panel::show);
      }
    }
  }

  fn on_close(&mut self, tab: &mut DockTab) -> egui_dock::tab_viewer::OnCloseResponse {
    match tab {
      DockTab::Reachability => self.app.reachability = None,
      DockTab::Properties => self.app.properties = None,
      DockTab::Outline => {}
      DockTab::Selection => self.app.selection = crate::app::Selection::None,
      DockTab::Dsl => {}
      DockTab::Help => {}
    }
    egui_dock::tab_viewer::OnCloseResponse::Close
  }
}

/// Opens (computing fresh data) and focuses the reachability tab, or closes it if already open —
/// shared by the "Ver" menu checkbox and the inspector panel's button.
pub fn toggle_reachability(app: &mut PetriApp) {
  if app.reachability.is_some() {
    close_reachability(app);
  } else {
    app.reachability = Some(crate::reachability_panel::ReachabilityState::explore(
      &app.net,
    ));
    if app.dock.find_tab(&DockTab::Reachability).is_none() {
      app.dock.push_to_focused_leaf(DockTab::Reachability);
    }
  }
}

pub fn close_reachability(app: &mut PetriApp) {
  app.reachability = None;
  if let Some(path) = app.dock.find_tab(&DockTab::Reachability) {
    app.dock.remove_tab(path);
  }
}

pub fn toggle_properties(app: &mut PetriApp) {
  if app.properties.is_some() {
    close_properties(app);
  } else {
    app.properties = Some(crate::properties_panel::PropertiesState::compute(&app.net));
    if app.dock.find_tab(&DockTab::Properties).is_none() {
      app.dock.push_to_focused_leaf(DockTab::Properties);
    }
  }
}

pub fn close_properties(app: &mut PetriApp) {
  app.properties = None;
  if let Some(path) = app.dock.find_tab(&DockTab::Properties) {
    app.dock.remove_tab(path);
  }
}

/// The outline has no computed-data `Option` to gate (it just lists the live net), so
/// "open" is purely "present in the dock."
pub fn toggle_outline(app: &mut PetriApp) {
  if let Some(path) = app.dock.find_tab(&DockTab::Outline) {
    app.dock.remove_tab(path);
  } else {
    app.dock.push_to_focused_leaf(DockTab::Outline);
  }
}

/// The DSL text buffer lives directly on `app.dsl` (not an `Option`) since it's real document
/// state, persisted in the `.gpn` like positions/colors/notes — same presence-only toggle as
/// the outline, nothing to compute or clear on open/close.
pub fn toggle_dsl(app: &mut PetriApp) {
  if let Some(path) = app.dock.find_tab(&DockTab::Dsl) {
    app.dock.remove_tab(path);
  } else {
    app.dock.push_to_focused_leaf(DockTab::Dsl);
  }
}

/// The help panel has no computed-data `Option` to gate either (it's static text) — same
/// presence-only toggle as the outline.
pub fn toggle_help(app: &mut PetriApp) {
  if let Some(path) = app.dock.find_tab(&DockTab::Help) {
    app.dock.remove_tab(path);
  } else {
    app.dock.push_to_focused_leaf(DockTab::Help);
  }
}

/// Keeps the Selection tab's presence in sync with whether anything is actually selected —
/// appears the moment you pick something, disappears the moment you don't, without the user
/// having to open/close it by hand. Call once per frame, before the dock is drawn.
pub fn sync_selection_tab(app: &mut PetriApp) {
  let has_selection = !matches!(app.selection, crate::app::Selection::None);
  let tab_present = app.dock.find_tab(&DockTab::Selection).is_some();
  if has_selection && !tab_present {
    app.dock.push_to_focused_leaf(DockTab::Selection);
  } else if !has_selection && tab_present {
    if let Some(path) = app.dock.find_tab(&DockTab::Selection) {
      app.dock.remove_tab(path);
    }
  }
}
