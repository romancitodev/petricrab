use eframe::egui;
use iconflow::{Pack, Size, Style};

/// Registers the embedded Lucide icon font with the egui context. Call once at startup.
pub fn install(ctx: &egui::Context) {
  let mut fonts = egui::FontDefinitions::default();
  for asset in iconflow::fonts() {
    fonts.font_data.insert(
      asset.family.to_string(),
      std::sync::Arc::new(egui::FontData::from_static(asset.bytes)),
    );
    fonts.families.insert(
      egui::FontFamily::Name(asset.family.into()),
      vec![asset.family.to_string()],
    );
  }
  // The whole app reads as one monospaced "editor" surface — labels, buttons, menus, badges —
  // instead of switching fonts between chrome and content. Reuse egui's own bundled monospace
  // font (already registered under `FontFamily::Monospace`) for `Proportional` too, so every
  // `TextStyle` (which all default to `Proportional`) keeps its usual size but renders
  // monospaced, with no new font file to embed.
  if let Some(mono) = fonts.families.get(&egui::FontFamily::Monospace).cloned() {
    fonts.families.insert(egui::FontFamily::Proportional, mono);
  }
  ctx.set_fonts(fonts);
}

/// A Lucide icon glyph as `RichText`, ready to drop into a `Button`/`label`/atoms tuple —
/// including a `Window` title, via `(icon(...), "text")`, since egui 0.35's `IntoAtoms` lets any
/// widget builder take a tuple of atoms instead of a single string.
/// Falls back to the icon's own name if it isn't found in the pack (e.g. a typo), so a
/// missing icon degrades to readable text instead of a silent blank button.
pub fn icon(name: &'static str, size: f32) -> egui::RichText {
  match iconflow::try_icon(Pack::Lucide, name, Style::Regular, Size::Regular) {
    Ok(icon_ref) => {
      let ch = char::from_u32(icon_ref.codepoint).unwrap_or('?');
      egui::RichText::new(ch.to_string())
        .family(egui::FontFamily::Name(icon_ref.family.into()))
        .size(size)
    }
    Err(_) => egui::RichText::new(name).size(size * 0.7),
  }
}
