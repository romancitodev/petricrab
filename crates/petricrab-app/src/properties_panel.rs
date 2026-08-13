use eframe::egui;

use crate::analysis::{self, NetProperties};
use crate::editor::section_label;
use crate::icons;
use crate::model::PetriNet;
use crate::reachability_panel::marking_text;

pub struct PropertiesState {
  props: NetProperties,
  /// `net.fingerprint()` as of the last compute — `show()` compares against the live net each
  /// frame and recomputes on a mismatch, so editing the net while this panel is open doesn't
  /// leave it showing stale boundedness/liveness/reversibility results.
  fingerprint: u64,
}

/// Small colored status pill (icon + text) — the one categorical-status shape reused for
/// every boundedness/liveness/reversibility result, and for the "aprox." marker itself.
fn status_badge(ui: &mut egui::Ui, icon_name: &'static str, text: &str, color: egui::Color32) {
  egui::Frame::default()
    .fill(color.gamma_multiply(0.16))
    .corner_radius(8.0)
    .inner_margin(egui::Margin::symmetric(8, 3))
    .show(ui, |ui| {
      ui.horizontal(|ui| {
        ui.label(icons::icon(icon_name, 12.0).color(color));
        ui.colored_label(color, text);
      });
    });
}

impl PropertiesState {
  pub fn compute(net: &PetriNet) -> Self {
    Self {
      props: analysis::analyze(net, &net.marking()),
      fingerprint: net.fingerprint(),
    }
  }

  pub fn show(&mut self, ui: &mut egui::Ui, net: &PetriNet) {
    let fingerprint = net.fingerprint();
    if fingerprint != self.fingerprint {
      *self = Self::compute(net);
    }
    let props = &self.props;

    // Boundedness (Karp-Miller) always terminates, so it's shown place by place even when
    // some of them are unbounded, regardless of whether liveness/reversibility can run below.
    section_label(ui, "Acotamiento");
    ui.add_space(6.0);
    egui::Grid::new("boundedness_grid")
      .num_columns(2)
      .spacing([12.0, 4.0])
      .show(ui, |ui| {
        for &(place, boundedness) in &props.boundedness {
          ui.label(net.place_label(place));
          match boundedness {
            petricrab_core::Boundedness::Bounded(k) => {
              status_badge(ui, "check", &format!("k = {k}"), crate::theme::success());
            }
            petricrab_core::Boundedness::Unbounded => {
              status_badge(ui, "triangle-alert", "No acotado", ui.visuals().warn_fg_color);
            }
          }
          ui.end_row();
        }
      });
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
      let (icon_name, color) = if props.safe {
        ("check", crate::theme::success())
      } else {
        ("circle", ui.visuals().weak_text_color())
      };
      ui.label(icons::icon(icon_name, 13.0).color(color));
      ui.label(if props.safe {
        "Safe (1-acotada)"
      } else {
        "No safe (algún lugar supera 1 token, o no está acotado)"
      });
    });
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);

    let behavior = &props.behavior;

    if !behavior.precise {
      ui.horizontal_wrapped(|ui| {
        ui.label(icons::icon("triangle-alert", 14.0).color(ui.visuals().warn_fg_color));
        ui.colored_label(
          ui.visuals().warn_fg_color,
          "El net no está acotado: Liveness y Reversibilidad de acá para abajo son \
           aproximadas (calculadas sobre el grafo de coverability, no el espacio de \
           estados exacto). \"Repetible por siempre\" puede en realidad ser sólo \
           \"repetible arbitrariamente\".",
        );
      });
      ui.add_space(12.0);
    }

    section_label(ui, "Liveness");
    ui.add_space(6.0);
    if behavior.liveness.is_empty() {
      ui.weak("(sin transiciones)");
    }
    egui::Grid::new("liveness_grid")
      .num_columns(2)
      .spacing([12.0, 4.0])
      .show(ui, |ui| {
        for t in &behavior.liveness {
          ui.label(net.transition_label(t.transition));
          ui.horizontal(|ui| {
            let (icon_name, color) = liveness_icon(t.level);
            status_badge(ui, icon_name, liveness_label(t.level), color);
            if !behavior.precise {
              status_badge(ui, "triangle-alert", "aprox.", ui.visuals().warn_fg_color);
            }
          });
          ui.end_row();
          if !t.example.is_empty() {
            let path = t
              .example
              .iter()
              .map(|&e| net.transition_label(e))
              .collect::<Vec<_>>()
              .join(" → ");
            ui.label("");
            ui.weak(format!("ruta: {path}"));
            ui.end_row();
          }
        }
      });
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);

    section_label(ui, "Reversibilidad");
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
      let (icon_name, color) = if behavior.reversible {
        ("rotate-ccw", crate::theme::success())
      } else {
        ("circle", ui.visuals().weak_text_color())
      };
      ui.label(icons::icon(icon_name, 13.0).color(color));
      ui.label(if behavior.reversible {
        "Reversible: siempre se puede volver al marking inicial"
      } else {
        "No reversible — pero siempre se puede volver a alguno de estos home states"
      });
      if !behavior.precise {
        status_badge(ui, "triangle-alert", "aprox.", ui.visuals().warn_fg_color);
      }
    });
    ui.add_space(8.0);
    if behavior.precise {
      ui.weak(format!("Home states ({})", behavior.home_states.len()));
      if behavior.home_states.is_empty() {
        ui.weak("(ninguno)");
      }
    } else if behavior.home_states.is_empty() {
      ui.weak("Home states: no disponibles (net no acotado)");
    } else {
      ui.weak("Home states conocidos (puede haber más, net no acotado)");
    }
    for home in &behavior.home_states {
      ui.label(format!("• {}", marking_text(net, home)));
    }
  }
}

fn liveness_label(level: petricrab_core::Liveness) -> &'static str {
  match level {
    petricrab_core::Liveness::Dead => "muerta (L0)",
    petricrab_core::Liveness::PotentiallyFirable => "disparable (L1)",
    petricrab_core::Liveness::ArbitrarilyRepeatable => "repetible arbitrariamente (L2)",
    petricrab_core::Liveness::RepeatableForever => "repetible por siempre (L3)",
    petricrab_core::Liveness::Total => "live (L4)",
  }
}

fn liveness_icon(level: petricrab_core::Liveness) -> (&'static str, egui::Color32) {
  match level {
    petricrab_core::Liveness::Dead => ("ban", crate::theme::danger()),
    petricrab_core::Liveness::Total => ("zap", crate::theme::success()),
    _ => ("circle", crate::theme::warning()),
  }
}
