//! Catppuccin Mocha theme for aml-gui.
//!
//! Turns egui's flat default dark into a cohesive, blended look: soft surface
//! fills, rounded widgets, an accent selection colour, and translucent tints for
//! status/callout surfaces (no opaque dark slabs — things blend, not box).
//! Palette matches the terminal daily-driver so the tool feels of a piece.

use egui::{Color32, CornerRadius, Stroke, Visuals};

// Catppuccin Mocha palette.
pub const BASE: Color32 = Color32::from_rgb(30, 30, 46);
pub const MANTLE: Color32 = Color32::from_rgb(24, 24, 37);
pub const CRUST: Color32 = Color32::from_rgb(17, 17, 27);
pub const SURFACE0: Color32 = Color32::from_rgb(49, 50, 68);
pub const SURFACE1: Color32 = Color32::from_rgb(69, 71, 90);
pub const SURFACE2: Color32 = Color32::from_rgb(88, 91, 112);
pub const TEXT: Color32 = Color32::from_rgb(205, 214, 244);
pub const SUBTEXT: Color32 = Color32::from_rgb(166, 173, 200);
pub const OVERLAY: Color32 = Color32::from_rgb(127, 132, 156);
pub const GREEN: Color32 = Color32::from_rgb(166, 227, 161);
pub const RED: Color32 = Color32::from_rgb(243, 139, 168);
pub const BLUE: Color32 = Color32::from_rgb(137, 180, 250);
pub const MAUVE: Color32 = Color32::from_rgb(203, 166, 247);
pub const PEACH: Color32 = Color32::from_rgb(250, 179, 135);

/// The same colour at a lower opacity — for blended fills over the panel.
pub fn tint(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

const RADIUS: CornerRadius = CornerRadius::same(5);

/// Build the app's `Visuals` (call once at startup).
pub fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = BASE;
    v.window_fill = MANTLE;
    v.window_stroke = Stroke::new(1.0, SURFACE0);
    v.extreme_bg_color = CRUST; // scroll / text-edit backgrounds
    v.faint_bg_color = tint(SURFACE0, 90); // striped rows — subtle
    v.code_bg_color = MANTLE;
    v.hyperlink_color = BLUE;
    v.selection.bg_fill = tint(BLUE, 70);
    v.selection.stroke = Stroke::new(1.0, BLUE);

    let w = &mut v.widgets;
    // Labels, separators, panel chrome.
    w.noninteractive.bg_fill = BASE;
    w.noninteractive.weak_bg_fill = BASE;
    w.noninteractive.bg_stroke = Stroke::new(1.0, SURFACE0);
    w.noninteractive.fg_stroke = Stroke::new(1.0, SUBTEXT);
    w.noninteractive.corner_radius = RADIUS;
    // Buttons at rest — lift clear of the panel base so affordance reads.
    w.inactive.bg_fill = SURFACE1;
    w.inactive.weak_bg_fill = SURFACE1;
    w.inactive.bg_stroke = Stroke::new(1.0, SURFACE2);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = RADIUS;
    // Hover.
    w.hovered.bg_fill = SURFACE1;
    w.hovered.weak_bg_fill = SURFACE1;
    w.hovered.bg_stroke = Stroke::new(1.0, OVERLAY);
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.corner_radius = RADIUS;
    // Pressed.
    w.active.bg_fill = SURFACE2;
    w.active.weak_bg_fill = SURFACE2;
    w.active.bg_stroke = Stroke::new(1.0, BLUE);
    w.active.fg_stroke = Stroke::new(1.0, TEXT);
    w.active.corner_radius = RADIUS;
    // Open combo / menu.
    w.open.bg_fill = SURFACE0;
    w.open.weak_bg_fill = SURFACE0;
    w.open.bg_stroke = Stroke::new(1.0, OVERLAY);
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    w.open.corner_radius = RADIUS;

    v
}

/// A subtle translucent "card" frame for grouping a section, blended over the
/// panel rather than a hard box.
pub fn card() -> egui::Frame {
    egui::Frame::NONE
        .fill(tint(SURFACE0, 60))
        .stroke(Stroke::new(1.0, tint(SURFACE1, 90)))
        .corner_radius(RADIUS)
        .inner_margin(8.0)
}

/// A translucent callout tinted by `accent` (e.g. red for conflicts).
pub fn callout(accent: Color32) -> egui::Frame {
    egui::Frame::NONE
        .fill(tint(accent, 26))
        .stroke(Stroke::new(1.0, tint(accent, 110)))
        .corner_radius(RADIUS)
        .inner_margin(8.0)
}

/// A small rounded status pill (translucent fill + coloured text).
pub fn pill(ui: &mut egui::Ui, text: &str, accent: Color32) -> egui::Response {
    egui::Frame::NONE
        .fill(tint(accent, 30))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(accent).small().strong());
        })
        .response
}
