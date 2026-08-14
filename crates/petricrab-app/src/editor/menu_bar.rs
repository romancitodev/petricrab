use eframe::egui;

use crate::app::{PetriApp, Selection};
use crate::icons;
use crate::theme;

use super::canvas::{begin_flat_menu, menu_item, reset_view, toggle_menu_item};
use super::clipboard::{copy_selection, paste_clipboard};
use super::history::{redo, undo};

const FILE_EXTENSION: &str = "gpn";
const MAX_RECENT: usize = 8;

/// Moves `path` to the front of `app.recent`, deduplicating and capping its length. Persisted
/// to disk by eframe's own storage (`PetriApp::save`), on its regular save cycle and on exit.
fn remember_recent(app: &mut PetriApp, path: std::path::PathBuf) {
  app.recent.retain(|p| p != &path);
  app.recent.insert(0, path);
  app.recent.truncate(MAX_RECENT);
}

/// Resets `app` to a blank, never-saved project (positions, view, undo history — everything
/// except the recent-files list).
fn file_new(app: &mut PetriApp) {
  let recent = std::mem::take(&mut app.recent);
  *app = PetriApp::new();
  app.recent = recent;
}

fn file_display_name(path: &std::path::Path) -> std::borrow::Cow<'_, str> {
  path
    .file_name()
    .map(|n| n.to_string_lossy())
    .unwrap_or_else(|| path.to_string_lossy())
}

fn open_path(app: &mut PetriApp, path: std::path::PathBuf) {
  match crate::project::load(&path) {
    Ok(loaded) => {
      let recent = std::mem::take(&mut app.recent);
      *app = PetriApp::new();
      app.recent = recent;
      app.net = loaded.net;
      app.positions = loaded.positions;
      app.rotation = loaded.rotation;
      app.colors = loaded.colors;
      app.notes = loaded.notes;
      app.next_place_n = loaded.next_place_n;
      app.next_transition_n = loaded.next_transition_n;
      log::info!("opened project: {}", path.display());
      app.notify(
        egui_toast::ToastKind::Success,
        format!("Abierto: {}", file_display_name(&path)),
      );
      remember_recent(app, path.clone());
      app.file_path = Some(path);
    }
    Err(e) => {
      log::error!("failed to open project {}: {e}", path.display());
      app.notify(
        egui_toast::ToastKind::Error,
        format!("No se pudo abrir el proyecto: {e}"),
      );
    }
  }
}

fn file_open(app: &mut PetriApp) {
  let Some(path) = rfd::FileDialog::new()
    .add_filter("gpn", &[FILE_EXTENSION])
    .pick_file()
  else {
    return;
  };
  open_path(app, path);
}

fn file_save_as(app: &mut PetriApp) {
  let Some(path) = rfd::FileDialog::new()
    .add_filter("gpn", &[FILE_EXTENSION])
    .set_file_name(format!("net.{FILE_EXTENSION}"))
    .save_file()
  else {
    return;
  };
  match crate::project::save(app, &path) {
    Ok(()) => {
      log::info!("saved project: {}", path.display());
      app.notify(
        egui_toast::ToastKind::Success,
        format!("Guardado: {}", file_display_name(&path)),
      );
      remember_recent(app, path.clone());
      app.file_path = Some(path);
    }
    Err(e) => {
      log::error!("failed to save project {}: {e}", path.display());
      app.notify(
        egui_toast::ToastKind::Error,
        format!("No se pudo guardar el proyecto: {e}"),
      );
    }
  }
}

fn file_save(app: &mut PetriApp) {
  match app.file_path.clone() {
    Some(path) => match crate::project::save(app, &path) {
      Ok(()) => {
        log::info!("saved project: {}", path.display());
        app.notify(
          egui_toast::ToastKind::Success,
          format!("Guardado: {}", file_display_name(&path)),
        );
        remember_recent(app, path);
      }
      Err(e) => {
        log::error!("failed to save project {}: {e}", path.display());
        app.notify(
          egui_toast::ToastKind::Error,
          format!("No se pudo guardar el proyecto: {e}"),
        );
      }
    },
    None => file_save_as(app),
  }
}

/// Top menu bar: identity mark on the left, then File/Edit/View menus.
pub fn menu_bar(app: &mut PetriApp, ui: &mut egui::Ui, ctx: &egui::Context) {
  let no_focus = ctx.memory(|m| m.focused().is_none());
  if no_focus && ctx.input(|i| i.key_pressed(egui::Key::F1)) {
    crate::dock::toggle_help(app);
  }

  egui::MenuBar::new().ui(ui, |ui| {
    // `MenuBar` forces a cramped `(2, 0)` button padding on its direct contents (fine for a
    // dense app menu bar in general, but reads as squished here) — give the top-level
    // Archivo/Editar/Ver openers and the theme toggle some breathing room instead.
    ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
    ui.horizontal(|ui| {
      ui.label(icons::icon("workflow", 16.0).color(theme::accent()));
      ui.add_space(2.0);
      ui.label(egui::RichText::new("petricrab").strong().size(14.0));
      ui.weak(concat!("v", env!("CARGO_PKG_VERSION")));
    });
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(4.0);

    ui.menu_button("Archivo", |ui| {
      ui.set_min_width(170.0);
      begin_flat_menu(ui);
      if menu_item(ui, "file-plus", "Nuevo").clicked() {
        file_new(app);
        ui.close();
      }
      if menu_item(ui, "folder-open", "Abrir…").clicked() {
        file_open(app);
        ui.close();
      }
      if menu_item(ui, "save", "Guardar").clicked() {
        file_save(app);
        ui.close();
      }
      if menu_item(ui, "save", "Guardar como…").clicked() {
        file_save_as(app);
        ui.close();
      }
      ui.separator();
      ui.add_enabled_ui(!app.recent.is_empty(), |ui| {
        ui.menu_button("Recientes", |ui| {
          ui.set_min_width(220.0);
          if app.recent.is_empty() {
            ui.weak("(ninguno)");
          } else {
            for path in app.recent.clone() {
              let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
              if ui
                .add(egui::Button::new(name).frame(false))
                .on_hover_text(path.to_string_lossy())
                .clicked()
              {
                open_path(app, path);
                ui.close();
              }
            }
          }
        });
      });
      ui.separator();
      if menu_item(ui, "log-out", "Salir").clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
      }
    });

    ui.menu_button("Editar", |ui| {
      ui.set_min_width(170.0);
      begin_flat_menu(ui);
      if ui
        .add_enabled(
          !app.undo_stack.is_empty(),
          egui::Button::new((icons::icon("undo-2", 13.0), "Deshacer"))
            .corner_radius(6.0)
            .frame_when_inactive(false),
        )
        .on_hover_text("Ctrl+Z")
        .clicked()
      {
        undo(app);
        ui.close();
      }
      if ui
        .add_enabled(
          !app.redo_stack.is_empty(),
          egui::Button::new((icons::icon("redo-2", 13.0), "Rehacer"))
            .corner_radius(6.0)
            .frame_when_inactive(false),
        )
        .on_hover_text("Ctrl+Shift+Z")
        .clicked()
      {
        redo(app);
        ui.close();
      }
      ui.separator();
      if ui
        .add_enabled(
          matches!(&app.selection, Selection::Nodes(n) if !n.is_empty()),
          egui::Button::new((icons::icon("copy", 13.0), "Copiar"))
            .corner_radius(6.0)
            .frame_when_inactive(false),
        )
        .on_hover_text("Ctrl+C")
        .clicked()
      {
        copy_selection(app, ui.ctx());
        ui.close();
      }
      if ui
        .add_enabled(
          app.clipboard.is_some(),
          egui::Button::new((icons::icon("clipboard-paste", 13.0), "Pegar"))
            .corner_radius(6.0)
            .frame_when_inactive(false),
        )
        .on_hover_text("Ctrl+V")
        .clicked()
      {
        paste_clipboard(app);
        ui.close();
      }
    });

    // Rect captured for the tutorial's last step to spotlight this button specifically instead
    // of the whole menu bar — set every frame, read only while a tutorial is open. "Propiedades
    // del net" lives under "Ver", not "Editar".
    app.menu_ver_rect = ui
      .menu_button("Ver", |ui| {
        ui.set_min_width(190.0);
        begin_flat_menu(ui);
        if toggle_menu_item(ui, app.show_grid, "Mostrar grilla").clicked() {
          app.show_grid = !app.show_grid;
        }
        if menu_item(ui, "locate-fixed", "Reiniciar vista").clicked() {
          reset_view(app);
          ui.close();
        }
        ui.separator();
        if toggle_menu_item(
          ui,
          app.reachability.is_some(),
          "Explorar espacio de estados",
        )
        .clicked()
        {
          crate::dock::toggle_reachability(app);
        }
        if toggle_menu_item(ui, app.properties.is_some(), "Propiedades del net").clicked() {
          crate::dock::toggle_properties(app);
        }
        let show_outline = app.dock.find_tab(&crate::dock::DockTab::Outline).is_some();
        if toggle_menu_item(ui, show_outline, "Estructura").clicked() {
          crate::dock::toggle_outline(app);
        }
      })
      .response
      .rect;

    ui.menu_button("Ayuda", |ui| {
      ui.set_min_width(190.0);
      begin_flat_menu(ui);
      let show_help = app.dock.find_tab(&crate::dock::DockTab::Help).is_some();
      if toggle_menu_item(ui, show_help, "Ayuda (F1)").clicked() {
        crate::dock::toggle_help(app);
      }
      ui.separator();
      if menu_item(ui, "graduation-cap", "Ver tutorial").clicked() {
        app.tutorial = Some(crate::tutorial::TutorialState::new());
        ui.close();
      }
    });

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
      let icon_name = if app.light_mode { "moon" } else { "sun" };
      let tooltip = if app.light_mode {
        "Cambiar a modo oscuro"
      } else {
        "Cambiar a modo claro"
      };
      if ui
        .add(egui::Button::new(icons::icon(icon_name, 15.0)).corner_radius(6.0))
        .on_hover_text(tooltip)
        .clicked()
      {
        app.light_mode = !app.light_mode;
        theme::set_light(ctx, app.light_mode);
      }
    });
  });
}
