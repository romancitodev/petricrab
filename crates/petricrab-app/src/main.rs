mod analysis;
mod app;
mod editor;
mod icons;
mod model;
mod properties_panel;
mod reachability_panel;

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "petricrab",
        eframe::NativeOptions::default(),
        Box::new(|cc| {
            icons::install(&cc.egui_ctx);
            Ok(Box::new(app::PetriApp::new()))
        }),
    )
}
