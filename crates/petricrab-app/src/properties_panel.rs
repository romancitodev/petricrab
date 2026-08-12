use eframe::egui;

use crate::analysis::{self, NetProperties};
use crate::editor::{card, section_label};
use crate::icons;
use crate::model::PetriNet;
use crate::reachability_panel::marking_text;

pub struct PropertiesState {
  props: NetProperties,
}

impl PropertiesState {
  pub fn compute(net: &PetriNet) -> Self {
    Self {
      props: analysis::analyze(net, &net.marking()),
    }
  }

  pub fn show(&self, ui: &mut egui::Ui, net: &PetriNet) {
    let props = &self.props;

    // Boundedness (Karp-Miller) always terminates, so it's shown place by place even when
    // some of them are unbounded, regardless of whether liveness/reversibility can run below.
    section_label(ui, "Acotamiento");
    ui.add_space(6.0);
    card(ui, |ui| {
      for &(place, boundedness) in &props.boundedness {
        match boundedness {
          petricrab_core::Boundedness::Bounded(k) => {
            ui.label(format!("{} (k = {k})", net.place_label(place)));
          }
          petricrab_core::Boundedness::Unbounded => {
            ui.colored_label(
              ui.visuals().warn_fg_color,
              format!("{}: no acotado", net.place_label(place)),
            );
          }
        }
      }
      ui.add_space(4.0);
      ui.horizontal(|ui| {
        let (icon_name, color) = if props.safe {
          ("check", crate::theme::SUCCESS)
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
    });
    ui.add_space(10.0);

    let behavior = &props.behavior;

    if !behavior.precise {
      card(ui, |ui| {
        ui.horizontal(|ui| {
          ui.label(icons::icon("triangle-alert", 14.0).color(ui.visuals().warn_fg_color));
          ui.colored_label(
            ui.visuals().warn_fg_color,
            "El net no está acotado: Liveness y Reversibilidad de acá para abajo son \
             aproximadas (calculadas sobre el grafo de coverability, no el espacio de \
             estados exacto). \"Repetible por siempre\" puede en realidad ser sólo \
             \"repetible arbitrariamente\".",
          );
        });
      });
      ui.add_space(10.0);
    }

    section_label(ui, "Liveness");
    ui.add_space(6.0);
    card(ui, |ui| {
      if behavior.liveness.is_empty() {
        ui.weak("(sin transiciones)");
      }
      for t in &behavior.liveness {
        ui.horizontal(|ui| {
          let (icon_name, color) = liveness_icon(t.level);
          ui.label(icons::icon(icon_name, 13.0).color(color));
          ui.label(format!(
            "{} — {}",
            net.transition_label(t.transition),
            liveness_label(t.level)
          ));
        });
        if !t.example.is_empty() {
          let path = t
            .example
            .iter()
            .map(|&e| net.transition_label(e))
            .collect::<Vec<_>>()
            .join(" → ");
          ui.horizontal(|ui| {
            ui.add_space(19.0); // align under the label, past the icon
            ui.weak(format!("ruta: {path}"));
          });
        }
      }
    });
    ui.add_space(10.0);

    section_label(ui, "Reversibilidad");
    ui.add_space(6.0);
    card(ui, |ui| {
      ui.horizontal(|ui| {
        let (icon_name, color) = if behavior.reversible {
          ("rotate-ccw", crate::theme::SUCCESS)
        } else {
          ("circle", ui.visuals().weak_text_color())
        };
        ui.label(icons::icon(icon_name, 13.0).color(color));
        ui.label(if behavior.reversible {
          "Reversible: siempre se puede volver al marking inicial"
        } else {
          "No reversible — pero siempre se puede volver a alguno de estos home states"
        });
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
    });
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
    petricrab_core::Liveness::Dead => ("ban", crate::theme::DANGER),
    petricrab_core::Liveness::Total => ("zap", crate::theme::SUCCESS),
    _ => ("circle", crate::theme::WARNING),
  }
}
