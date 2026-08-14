use eframe::egui;

/// Condensed in-app reference, adapted from `docs/GUIA_DE_USO.md` for a narrow docked panel
/// instead of a full manual — one `CollapsingHeader` per topic so it doesn't turn into a wall
/// of text. Hardcoded, not loaded from the `.md` file: this is fixed text that ships with the
/// binary, not a doc-rendering system.
pub fn show(ui: &mut egui::Ui) {
  egui::CollapsingHeader::new("Navegación y modos")
    .default_open(true)
    .show(ui, |ui| {
      ui.label("Espacio + arrastrar: mover la vista. Rueda: zoom.");
      ui.add_space(4.0);
      ui.label("V — Seleccionar / token game");
      ui.label("P — Agregar place");
      ui.label("T — Agregar transition");
      ui.label("C — Conectar (arco)");
      ui.label("N — Agregar nota");
      ui.add_space(4.0);
      ui.label(
        "Supr / Backspace borra la selección. Click derecho abre el menú contextual del elemento.",
      );
    });

  egui::CollapsingHeader::new("Tipos de arco").show(ui, |ui| {
    ui.label("Consume (flecha llena): resta/suma tokens al disparar, peso 1–99.");
    ui.label("Peek (flecha doble): requiere tokens pero no los consume, peso fijo 1.");
    ui.label("Inhibit (círculo hueco): bloquea la transition mientras haya tokens, peso fijo 1.");
    ui.add_space(4.0);
    ui.label("Se cambia el tipo desde el panel de Selección, con el arco elegido.");
  });

  egui::CollapsingHeader::new("Simulación (token game)").show(ui, |ui| {
    ui.label("En modo Seleccionar, click sobre una transition habilitada la dispara.");
    ui.label("El popup de simulación (ícono play) trae marking, transiciones habilitadas, paso atrás/reiniciar/adelante.");
  });

  egui::CollapsingHeader::new("Análisis de la red").show(ui, |ui| {
    ui.label("Acotamiento: si algún place puede crecer sin límite de tokens.");
    ui.label("Liveness: nivel L0–L4 (Murata) por transition.");
    ui.label("Reversibilidad: si existen home states a los que siempre se puede volver.");
    ui.label("Deadlocks: marcados sin ninguna transition habilitada.");
    ui.add_space(4.0);
    ui.label("Cualquier resultado con una secuencia testigo trae un botón \"Ver ruta\" que la reproduce sobre el canvas real.");
  });

  egui::CollapsingHeader::new("Proyectos (.gpn)").show(ui, |ui| {
    ui.label("Archivo → Guardar / Guardar como… / Abrir…, formato binario propio (rkyv).");
    ui.label("Nuevo y Abrir… reemplazan el net actual sin avisar si hay cambios sin guardar.");
    ui.label("Un .gpn de otra versión de formato no carga (sin migración automática todavía).");
  });
}
