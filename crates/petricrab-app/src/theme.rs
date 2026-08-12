use eframe::egui;

/// Neutral zinc grays, no hue accent. Selected/active state is brightness, not color.
pub const INK: egui::Color32 = egui::Color32::from_rgb(9, 9, 11);
pub const SURFACE: egui::Color32 = egui::Color32::from_rgb(24, 24, 27);
pub const SURFACE_RAISED: egui::Color32 = egui::Color32::from_rgb(39, 39, 42);
pub const SURFACE_HOVER: egui::Color32 = egui::Color32::from_rgb(63, 63, 70);
pub const LINE: egui::Color32 = egui::Color32::from_rgb(46, 46, 51);
pub const LINE_STRONG: egui::Color32 = egui::Color32::from_rgb(82, 82, 91);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(244, 244, 245);
pub const TEXT_STRONG: egui::Color32 = egui::Color32::from_rgb(255, 255, 255);
pub const TEXT_WEAK: egui::Color32 = egui::Color32::from_rgb(161, 161, 170);

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(228, 228, 231);
pub const SUCCESS: egui::Color32 = egui::Color32::from_rgb(109, 186, 142);
pub const DANGER: egui::Color32 = egui::Color32::from_rgb(200, 114, 118);
pub const WARNING: egui::Color32 = egui::Color32::from_rgb(193, 166, 109);

/// Corner radius shared by all the floating chrome (windows, toolbar pill, simulate popup).
pub const RADIUS_LG: f32 = 12.0;

/// Call once at startup, before the first frame. Locks the app to dark mode.
pub fn apply(ctx: &egui::Context) {
  ctx.set_theme(egui::Theme::Dark);
  ctx.style_mut_of(egui::Theme::Dark, |style| {
    let mut visuals = egui::Visuals::dark();

    visuals.weak_text_color = Some(TEXT_WEAK);
    visuals.hyperlink_color = ACCENT;
    visuals.faint_bg_color = SURFACE_RAISED;
    visuals.extreme_bg_color = INK;
    visuals.code_bg_color = SURFACE_RAISED;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = DANGER;

    visuals.window_corner_radius = egui::CornerRadius::same(RADIUS_LG as u8);
    visuals.window_fill = SURFACE;
    visuals.window_stroke = egui::Stroke::new(1.0, LINE);
    visuals.window_shadow = egui::Shadow {
      offset: [0, 4],
      blur: 12,
      spread: 0,
      color: egui::Color32::from_black_alpha(90),
    };
    visuals.menu_corner_radius = egui::CornerRadius::same(8);
    visuals.panel_fill = SURFACE;
    visuals.popup_shadow = egui::Shadow {
      offset: [0, 3],
      blur: 8,
      spread: 0,
      color: egui::Color32::from_black_alpha(80),
    };

    // Also drives Button::selected (toolbar active-mode highlight).
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = egui::Stroke::new(1.0, INK);

    visuals.widgets.noninteractive = egui::style::WidgetVisuals {
      weak_bg_fill: SURFACE,
      bg_fill: SURFACE,
      bg_stroke: egui::Stroke::new(1.0, LINE),
      fg_stroke: egui::Stroke::new(1.0, TEXT),
      corner_radius: egui::CornerRadius::same(6),
      expansion: 0.0,
    };
    visuals.widgets.inactive = egui::style::WidgetVisuals {
      weak_bg_fill: SURFACE_RAISED,
      bg_fill: SURFACE_RAISED,
      bg_stroke: egui::Stroke::new(1.0, LINE),
      fg_stroke: egui::Stroke::new(1.0, TEXT),
      corner_radius: egui::CornerRadius::same(6),
      expansion: 0.0,
    };
    visuals.widgets.hovered = egui::style::WidgetVisuals {
      weak_bg_fill: SURFACE_HOVER,
      bg_fill: SURFACE_HOVER,
      bg_stroke: egui::Stroke::new(1.0, LINE_STRONG),
      fg_stroke: egui::Stroke::new(1.0, TEXT_STRONG),
      corner_radius: egui::CornerRadius::same(6),
      expansion: 0.0,
    };
    // fg_stroke here also feeds Visuals::strong_text_color(), used by every `.strong()` label
    // in the app. Keep it bright, not just legible against bg_fill while pressed.
    visuals.widgets.active = egui::style::WidgetVisuals {
      weak_bg_fill: ACCENT,
      bg_fill: ACCENT,
      bg_stroke: egui::Stroke::new(1.0, ACCENT),
      fg_stroke: egui::Stroke::new(1.2, TEXT_STRONG),
      corner_radius: egui::CornerRadius::same(6),
      expansion: 0.0,
    };
    visuals.widgets.open = egui::style::WidgetVisuals {
      weak_bg_fill: SURFACE_RAISED,
      bg_fill: SURFACE_RAISED,
      bg_stroke: egui::Stroke::new(1.0, ACCENT.gamma_multiply(0.5)),
      fg_stroke: egui::Stroke::new(1.0, TEXT_STRONG),
      corner_radius: egui::CornerRadius::same(6),
      expansion: 0.0,
    };

    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.indent = 16.0;
    style.animation_time = 0.12;
  });
}
