use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;
use egui::Color32;

/// Neutral zinc grays, no hue accent. Selected/active state is brightness, not color. One
/// instance for each theme; light is the same zinc ramp mirrored around its midpoint (e.g.
/// `ink`/darkest-in-dark becomes lightest-in-light), not a different color system.
struct Palette {
  ink: Color32,
  surface: Color32,
  surface_raised: Color32,
  surface_hover: Color32,
  line: Color32,
  line_strong: Color32,
  text: Color32,
  text_strong: Color32,
  text_weak: Color32,
  accent: Color32,
  success: Color32,
  danger: Color32,
  warning: Color32,
}

const DARK: Palette = Palette {
  ink: Color32::from_rgb(9, 9, 11),
  surface: Color32::from_rgb(24, 24, 27),
  surface_raised: Color32::from_rgb(39, 39, 42),
  surface_hover: Color32::from_rgb(63, 63, 70),
  line: Color32::from_rgb(46, 46, 51),
  line_strong: Color32::from_rgb(82, 82, 91),
  text: Color32::from_rgb(244, 244, 245),
  text_strong: Color32::from_rgb(255, 255, 255),
  text_weak: Color32::from_rgb(161, 161, 170),
  accent: Color32::from_rgb(228, 228, 231),
  success: Color32::from_rgb(109, 186, 142),
  danger: Color32::from_rgb(200, 114, 118),
  warning: Color32::from_rgb(193, 166, 109),
};

const LIGHT: Palette = Palette {
  ink: Color32::from_rgb(250, 250, 251),
  surface: Color32::from_rgb(244, 244, 245),
  surface_raised: Color32::from_rgb(228, 228, 231),
  surface_hover: Color32::from_rgb(212, 212, 216),
  line: Color32::from_rgb(219, 219, 223),
  line_strong: Color32::from_rgb(161, 161, 170),
  text: Color32::from_rgb(24, 24, 27),
  text_strong: Color32::from_rgb(9, 9, 11),
  text_weak: Color32::from_rgb(113, 113, 122),
  accent: Color32::from_rgb(39, 39, 42),
  success: Color32::from_rgb(109, 186, 142),
  danger: Color32::from_rgb(200, 114, 118),
  warning: Color32::from_rgb(193, 166, 109),
};

/// Which palette custom-painted code (canvas, graph, badges — anything not driven by egui's own
/// `Visuals`) should read this frame. Set once at startup and again on every toggle in `apply`
/// / `set_light`; read via the `theme::ink()`-style accessors below.
static IS_LIGHT: AtomicBool = AtomicBool::new(false);

pub fn is_light() -> bool {
  IS_LIGHT.load(Ordering::Relaxed)
}

fn current() -> &'static Palette {
  if is_light() { &LIGHT } else { &DARK }
}

pub fn ink() -> Color32 {
  current().ink
}
pub fn surface() -> Color32 {
  current().surface
}
pub fn surface_raised() -> Color32 {
  current().surface_raised
}
pub fn surface_hover() -> Color32 {
  current().surface_hover
}
pub fn line_strong() -> Color32 {
  current().line_strong
}
pub fn text() -> Color32 {
  current().text
}
pub fn text_strong() -> Color32 {
  current().text_strong
}
pub fn text_weak() -> Color32 {
  current().text_weak
}
pub fn accent() -> Color32 {
  current().accent
}
pub fn success() -> Color32 {
  current().success
}
pub fn danger() -> Color32 {
  current().danger
}
pub fn warning() -> Color32 {
  current().warning
}

/// Corner radius shared by all the floating chrome (windows, toolbar pill, simulate popup).
pub const RADIUS_LG: f32 = 12.0;

fn build_visuals(dark_base: bool, palette: &Palette) -> egui::Visuals {
  let mut visuals = if dark_base {
    egui::Visuals::dark()
  } else {
    egui::Visuals::light()
  };

  visuals.weak_text_color = Some(palette.text_weak);
  visuals.hyperlink_color = palette.accent;
  visuals.faint_bg_color = palette.surface_raised;
  visuals.extreme_bg_color = palette.ink;
  visuals.code_bg_color = palette.surface_raised;
  visuals.warn_fg_color = palette.warning;
  visuals.error_fg_color = palette.danger;

  visuals.window_corner_radius = egui::CornerRadius::same(RADIUS_LG as u8);
  visuals.window_fill = palette.surface;
  visuals.window_stroke = egui::Stroke::new(1.0, palette.line);
  visuals.window_shadow = egui::Shadow {
    offset: [0, 4],
    blur: 12,
    spread: 0,
    color: egui::Color32::from_black_alpha(if dark_base { 90 } else { 40 }),
  };
  visuals.menu_corner_radius = egui::CornerRadius::same(8);
  visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
  visuals.panel_fill = palette.surface;
  visuals.popup_shadow = egui::Shadow {
    offset: [0, 3],
    blur: 8,
    spread: 0,
    color: egui::Color32::from_black_alpha(if dark_base { 80 } else { 30 }),
  };

  // Also drives Button::selected (toolbar active-mode highlight).
  visuals.selection.bg_fill = palette.accent;
  visuals.selection.stroke = egui::Stroke::new(1.0, palette.ink);

  visuals.widgets.noninteractive = egui::style::WidgetVisuals {
    weak_bg_fill: palette.surface,
    bg_fill: palette.surface,
    bg_stroke: egui::Stroke::new(1.0, palette.line),
    fg_stroke: egui::Stroke::new(1.0, palette.text),
    corner_radius: egui::CornerRadius::same(6),
    expansion: 0.0,
  };
  visuals.widgets.inactive = egui::style::WidgetVisuals {
    weak_bg_fill: palette.surface_raised,
    bg_fill: palette.surface_raised,
    bg_stroke: egui::Stroke::new(1.0, palette.line),
    fg_stroke: egui::Stroke::new(1.0, palette.text),
    corner_radius: egui::CornerRadius::same(6),
    expansion: 0.0,
  };
  visuals.widgets.hovered = egui::style::WidgetVisuals {
    weak_bg_fill: palette.surface_hover,
    bg_fill: palette.surface_hover,
    bg_stroke: egui::Stroke::new(1.0, palette.line_strong),
    fg_stroke: egui::Stroke::new(1.0, palette.text_strong),
    corner_radius: egui::CornerRadius::same(6),
    expansion: 0.0,
  };
  // fg_stroke here also feeds Visuals::strong_text_color(), used by every `.strong()` label
  // in the app. Keep it bright/dark (theme-appropriate extreme), not just legible against
  // bg_fill while pressed.
  visuals.widgets.active = egui::style::WidgetVisuals {
    weak_bg_fill: palette.accent,
    bg_fill: palette.accent,
    bg_stroke: egui::Stroke::new(1.0, palette.accent),
    fg_stroke: egui::Stroke::new(1.2, palette.text_strong),
    corner_radius: egui::CornerRadius::same(6),
    expansion: 0.0,
  };
  visuals.widgets.open = egui::style::WidgetVisuals {
    weak_bg_fill: palette.surface_raised,
    bg_fill: palette.surface_raised,
    bg_stroke: egui::Stroke::new(1.0, palette.accent.gamma_multiply(0.5)),
    fg_stroke: egui::Stroke::new(1.0, palette.text_strong),
    corner_radius: egui::CornerRadius::same(6),
    expansion: 0.0,
  };

  visuals
}

fn apply_spacing(style: &mut egui::Style) {
  style.spacing.item_spacing = egui::vec2(8.0, 8.0);
  style.spacing.button_padding = egui::vec2(12.0, 6.0);
  style.spacing.window_margin = egui::Margin::same(16);
  style.spacing.menu_margin = egui::Margin::same(6);
  style.spacing.indent = 16.0;
  style.animation_time = 0.12;
}

/// Call once at startup, before the first frame. Populates both the dark and light `Visuals`
/// slots (so a runtime toggle via `set_light` never needs to rebuild them) and activates
/// whichever `light` says — normally the persisted preference from last run.
pub fn apply(ctx: &egui::Context, light: bool) {
  ctx.style_mut_of(egui::Theme::Dark, |style| {
    style.visuals = build_visuals(true, &DARK);
    apply_spacing(style);
  });
  ctx.style_mut_of(egui::Theme::Light, |style| {
    style.visuals = build_visuals(false, &LIGHT);
    apply_spacing(style);
  });
  set_light(ctx, light);
}

/// Switches theme at runtime (e.g. the "Modo claro" menu checkbox). Both `Visuals` slots are
/// already built by `apply`, so this is just flipping which one is active.
pub fn set_light(ctx: &egui::Context, light: bool) {
  IS_LIGHT.store(light, Ordering::Relaxed);
  ctx.set_theme(if light {
    egui::Theme::Light
  } else {
    egui::Theme::Dark
  });
  // egui's own `Theme` only repaints what egui draws — the OS-native window titlebar (min/
  // max/close chrome) is drawn by Windows itself and stays whatever it was, so without this
  // it's stuck on a jarring black bar over a light-mode window. `SetTheme` asks winit to flip
  // the native frame's dark/light mode (DWM immersive dark mode on Windows) to match.
  ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(if light {
    egui::SystemTheme::Light
  } else {
    egui::SystemTheme::Dark
  }));
}
