mod analysis;
mod app;
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
    eframe::NativeOptions::default(),
    Box::new(|cc| {
      icons::install(&cc.egui_ctx);
      theme::apply(&cc.egui_ctx);
      let mut app = app::PetriApp::new();
      if let Some(storage) = cc.storage {
        let persisted: app::PersistedState =
          eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default();
        // Drop entries for files that moved/were deleted since the last run.
        app.recent = persisted.recent.into_iter().filter(|p| p.is_file()).collect();
      }
      Ok(Box::new(app))
    }),
  )
}
