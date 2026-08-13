mod analysis;
mod app;
mod dock;
mod editor;
mod icons;
mod model;
mod project;
mod properties_panel;
mod reachability_panel;
mod theme;

fn main() -> eframe::Result<()> {
  eframe::run_native(
    "petricrab",
    eframe::NativeOptions {
      // Without an explicit size, the window opens at the OS default then egui immediately
      // resizes it to fit content on frame 1 — that jump can read as "window closed and
      // reopened" rather than a resize.
      viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
      ..Default::default()
    },
    Box::new(|cc| {
      icons::install(&cc.egui_ctx);
      let mut app = app::PetriApp::new();
      let persisted = cc
        .storage
        .and_then(|storage| eframe::get_value::<app::PersistedState>(storage, eframe::APP_KEY))
        .unwrap_or_default();
      theme::apply(&cc.egui_ctx, persisted.light_mode);
      app.light_mode = persisted.light_mode;
      // Drop entries for files that moved/were deleted since the last run.
      app.recent = persisted.recent.into_iter().filter(|p| p.is_file()).collect();
      Ok(Box::new(app))
    }),
  )
}
