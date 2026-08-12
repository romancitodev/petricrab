use eframe::egui;

use crate::analysis::{self, NetProperties};
use crate::editor::{card, section_label};
use crate::icons;
use crate::model::PetriNet;
use crate::reachability_panel::marking_text;

pub enum PropertiesState {
    Computed(NetProperties),
    /// Karp-Miller proved these places unbounded — liveness/reversibility need a finite
    /// `R(M0)`, so there's nothing further to compute until the net itself changes.
    Unbounded(Vec<crate::model::PlaceId>),
}

impl PropertiesState {
    pub fn compute(net: &PetriNet) -> Self {
        match analysis::analyze(net, &net.marking()) {
            Ok(props) => Self::Computed(props),
            Err(unbounded) => Self::Unbounded(unbounded),
        }
    }

    pub fn show(&self, ui: &mut egui::Ui, net: &PetriNet) {
        let props = match self {
            PropertiesState::Unbounded(places) => {
                let names = places
                    .iter()
                    .map(|&p| net.place_label(p))
                    .collect::<Vec<_>>()
                    .join(", ");
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            icons::icon("triangle-alert", 14.0).color(ui.visuals().warn_fg_color),
                        );
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            format!("No acotado: {names} crece sin límite."),
                        );
                    });
                });
                return;
            }
            PropertiesState::Computed(props) => props,
        };

        section_label(ui, "Acotamiento");
        ui.add_space(6.0);
        card(ui, |ui| {
            for &(place, boundedness) in &props.boundedness {
                let text = match boundedness {
                    petricrab_core::Boundedness::Bounded(k) => format!("{} (k = {k})", net.place_label(place)),
                    petricrab_core::Boundedness::Unbounded => unreachable!("filtered out above"),
                };
                ui.label(text);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let (icon_name, color) = if props.safe {
                    ("check", egui::Color32::from_rgb(70, 165, 95))
                } else {
                    ("circle", ui.visuals().weak_text_color())
                };
                ui.label(icons::icon(icon_name, 13.0).color(color));
                ui.label(if props.safe {
                    "Safe (1-acotada)"
                } else {
                    "No safe (algún lugar supera 1 token)"
                });
            });
        });
        ui.add_space(10.0);

        section_label(ui, "Liveness");
        ui.add_space(6.0);
        card(ui, |ui| {
            if props.liveness.is_empty() {
                ui.weak("(sin transiciones)");
            }
            for t in &props.liveness {
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
                let (icon_name, color) = if props.reversible {
                    ("rotate-ccw", egui::Color32::from_rgb(70, 165, 95))
                } else {
                    ("circle", ui.visuals().weak_text_color())
                };
                ui.label(icons::icon(icon_name, 13.0).color(color));
                ui.label(if props.reversible {
                    "Reversible: siempre se puede volver al marking inicial"
                } else {
                    "No reversible — pero siempre se puede volver a alguno de estos home states"
                });
            });
            ui.add_space(8.0);
            ui.weak(format!("Home states ({})", props.home_states.len()));
            if props.home_states.is_empty() {
                ui.weak("(ninguno)");
            }
            for home in &props.home_states {
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
        petricrab_core::Liveness::Dead => ("ban", egui::Color32::from_rgb(224, 82, 82)),
        petricrab_core::Liveness::Total => ("zap", egui::Color32::from_rgb(70, 165, 95)),
        _ => ("circle", egui::Color32::from_rgb(220, 170, 60)),
    }
}
