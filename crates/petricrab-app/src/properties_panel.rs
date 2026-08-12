use eframe::egui;

use crate::analysis::{self, NetProperties};
use crate::editor::{card, section_label};
use crate::icons;
use crate::model::PetriNet;

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
            for &(t, level) in &props.liveness {
                ui.horizontal(|ui| {
                    let (icon_name, color) = liveness_icon(level);
                    ui.label(icons::icon(icon_name, 13.0).color(color));
                    ui.label(format!("{} — {}", net.transition_label(t), liveness_label(level)));
                });
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
                    "Reversible: siempre se puede volver al marking inicial".to_string()
                } else {
                    format!(
                        "No reversible, pero tiene {} home state(s): siempre se puede volver a alguno de ellos",
                        props.home_state_count
                    )
                });
            });
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
