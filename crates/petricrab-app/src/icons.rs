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
    ctx.set_fonts(fonts);
}

/// A Lucide icon glyph as `RichText`, ready to drop into a `Button`/`label`/atoms tuple.
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
