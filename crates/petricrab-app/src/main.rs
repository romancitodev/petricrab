// Windows only, no-op elsewhere: a debug build keeps the console (so println!/panics stay
// visible while developing), a release build drops it, so launching the .exe doesn't pop a
// terminal behind the window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod app;
mod dock;
mod dsl;
mod dsl_panel;
mod editor;
mod help_panel;
mod icons;
mod model;
mod project;
mod properties_panel;
mod reachability_panel;
mod route_modal;
mod theme;
mod tutorial;

/// Logs to `petricrab.log` next to the executable (append mode, so a previous crash's entries
/// survive until you go looking for them) instead of a console, which release builds no longer
/// open. Also routes panics through the same log via `log_panics`, so a crash leaves a trace
/// instead of just vanishing.
fn init_logging() {
  let log_path = std::env::current_exe()
    .ok()
    .and_then(|exe| exe.parent().map(|dir| dir.join("petricrab.log")))
    .unwrap_or_else(|| std::path::PathBuf::from("petricrab.log"));

  if let Ok(file) = std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(&log_path)
  {
    let _ = simplelog::WriteLogger::init(
      simplelog::LevelFilter::Info,
      simplelog::Config::default(),
      file,
    );
  }
  log_panics::init();
  log::info!(
    "petricrab v{} starting, log: {}",
    env!("CARGO_PKG_VERSION"),
    log_path.display()
  );
}

fn main() -> eframe::Result<()> {
  init_logging();
  eframe::run_native(
    "petricrab",
    eframe::NativeOptions {
      // Without an explicit size, the window opens at the OS default then egui immediately
      // resizes it to fit content on frame 1 — that jump can read as "window closed and
      // reopened" rather than a resize.
      viewport: eframe::egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_title(concat!("petricrab v", env!("CARGO_PKG_VERSION"))),
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
      app.recent = persisted
        .recent
        .into_iter()
        .filter(|p| p.is_file())
        .collect();
      app.tutorial_seen = persisted.tutorial_seen;
      if !app.tutorial_seen {
        app.tutorial = Some(tutorial::TutorialState::new());
      }
      Ok(Box::new(app))
    }),
  )
}
