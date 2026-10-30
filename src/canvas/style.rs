//! The neoworks look: Geist, a near-black (or warm white) page, soft cards with hairline borders,
//! and off-white pill buttons.

use std::sync::Arc;

use egui::{
    self, Align, Align2, Button, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Layout, Response, RichText, Sense, Shadow, Stroke, TextStyle, Ui, Visuals, vec2,
};

use crate::config::Theme;

const MEDIUM: &str = "geist-medium";

pub fn medium() -> FontFamily {
    FontFamily::Name(MEDIUM.into())
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub card: Color32,
    pub border: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub text: Color32,
    pub muted: Color32,
    /// Text on a primary (text-coloured) button.
    pub on_text: Color32,
    pub error: Color32,
}

impl Palette {
    pub fn of(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self {
                bg: Color32::from_rgb(13, 13, 13),
                card: Color32::from_rgb(20, 20, 20),
                border: Color32::from_rgb(38, 38, 38),
                hover: Color32::from_rgb(28, 28, 28),
                selected: Color32::from_rgb(34, 34, 34),
                text: Color32::from_rgb(242, 240, 237),
                muted: Color32::from_rgb(140, 138, 134),
                on_text: Color32::from_rgb(17, 17, 17),
                error: Color32::from_rgb(240, 120, 110),
            },
            Theme::Light => Self {
                bg: Color32::from_rgb(247, 247, 245),
                card: Color32::from_rgb(255, 255, 255),
                border: Color32::from_rgb(229, 227, 223),
                hover: Color32::from_rgb(242, 241, 238),
                selected: Color32::from_rgb(234, 233, 229),
                text: Color32::from_rgb(20, 20, 20),
                muted: Color32::from_rgb(110, 108, 104),
                on_text: Color32::from_rgb(250, 250, 248),
                error: Color32::from_rgb(190, 50, 40),
            },
        }
    }
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "geist".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Geist-Regular.ttf"))),
    );
    fonts.font_data.insert(
        MEDIUM.into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Geist-Medium.ttf"))),
    );
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "geist".into());
    // egui's own fonts stay behind Geist for symbols it lacks.
    let mut medium_stack = vec![MEDIUM.to_string()];
    medium_stack.extend(proportional.iter().cloned());
    fonts.families.insert(medium(), medium_stack);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context, theme: Theme) {
    let p = Palette::of(theme);
    let egui_theme = match theme {
        Theme::Dark => egui::Theme::Dark,
        Theme::Light => egui::Theme::Light,
    };

    let mut v = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.extreme_bg_color = p.bg;
    v.text_edit_bg_color = Some(p.bg);
    v.faint_bg_color = p.hover;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(12);
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.selection.bg_fill = p.text.gamma_multiply(0.2);
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.hyperlink_color = p.text;
    v.error_fg_color = p.error;

    let w = &mut v.widgets;
    for (state, fill, stroke) in [
        (&mut w.noninteractive, p.card, p.border),
        (&mut w.inactive, p.hover, p.border),
        (&mut w.hovered, p.selected, p.muted.gamma_multiply(0.6)),
        (&mut w.active, p.selected, p.muted),
        (&mut w.open, p.selected, p.border),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.fg_stroke = Stroke::new(1.0, p.text);
        state.corner_radius = CornerRadius::same(8);
        state.expansion = 0.0;
    }
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);

    ctx.set_visuals_of(egui_theme, v);
    ctx.style_mut_of(egui_theme, |style| {
        let proportional = |size| FontId::new(size, FontFamily::Proportional);
        style.text_styles = [
            (TextStyle::Heading, FontId::new(22.0, medium())),
            (TextStyle::Body, proportional(14.0)),
            (TextStyle::Button, proportional(14.0)),
            (TextStyle::Small, proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        style.spacing.item_spacing = vec2(8.0, 8.0);
        style.spacing.button_padding = vec2(12.0, 5.0);
        style.spacing.interact_size.y = 30.0;
    });
    ctx.set_theme(egui_theme);
}

/// A menu entry that opens a page: label on the left, current value and a chevron on the right.
pub fn nav_row(ui: &mut Ui, p: &Palette, label: &str, value: &str) -> Response {
    menu_row(ui, p, label, value, true)
}

/// A menu entry that changes its value in place when clicked.
pub fn value_row(ui: &mut Ui, p: &Palette, label: &str, value: &str) -> Response {
    menu_row(ui, p, label, value, false)
}

fn menu_row(ui: &mut Ui, p: &Palette, label: &str, value: &str, chevron: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 10.0, p.hover);
    }
    let painter = ui.painter();
    let font = FontId::new(14.0, FontFamily::Proportional);
    let y = rect.center().y;
    painter.text(egui::pos2(rect.left() + 10.0, y), Align2::LEFT_CENTER, label, font.clone(), p.text);
    let value_right = if chevron {
        let chevron = FontId::new(17.0, FontFamily::Proportional);
        painter.text(egui::pos2(rect.right() - 10.0, y), Align2::RIGHT_CENTER, "›", chevron, p.muted).left() - 8.0
    } else {
        rect.right() - 10.0
    };
    // Truncate long values so they never run into the label.
    let room = rect.width() - 30.0 - painter.layout_no_wrap(label.into(), font.clone(), p.text).size().x - 24.0;
    let mut value = value.to_string();
    while value.chars().count() > 2 && painter.layout_no_wrap(value.clone(), font.clone(), p.muted).size().x > room {
        // Drop the last character (and the previous ellipsis) and end with an ellipsis again.
        let keep = value.trim_end_matches('…').chars().count() - 1;
        value = value.chars().take(keep).collect::<String>() + "…";
    }
    painter.text(egui::pos2(value_right, y), Align2::RIGHT_CENTER, value, font, p.muted);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A page's title bar with a way back to the menu. Returns true when back was clicked.
pub fn page_header(ui: &mut Ui, p: &Palette, title: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 10.0, p.hover);
    }
    let y = rect.center().y;
    let painter = ui.painter();
    painter.text(egui::pos2(rect.left() + 10.0, y), Align2::LEFT_CENTER, "‹", FontId::new(17.0, FontFamily::Proportional), p.muted);
    painter.text(egui::pos2(rect.left() + 26.0, y), Align2::LEFT_CENTER, title, FontId::new(14.0, medium()), p.text);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A settings row: label on the left, control on the right.
pub fn row(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        ui.add_space(10.0);
        ui.label(RichText::new(label).color(p.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(10.0);
            add(ui);
        });
    });
}

pub fn divider(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range().shrink(2.0), rect.center().y, Stroke::new(1.0, p.border));
}

/// The off-white pill, like "Start for free".
pub fn primary(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    let label = RichText::new(text).family(medium()).color(p.on_text);
    ui.add(Button::new(label).fill(p.text).stroke(Stroke::NONE).corner_radius(255).min_size(vec2(0.0, 28.0)))
}

pub fn ghost(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    let label = RichText::new(text).color(p.text);
    ui.add(
        Button::new(label)
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(255)
            .min_size(vec2(0.0, 28.0)),
    )
}

/// A toggleable pill, filled when on.
pub fn chip(ui: &mut Ui, p: &Palette, text: &str, on: bool) -> Response {
    let (fill, color, stroke) = if on {
        (p.text, p.on_text, Stroke::NONE)
    } else {
        (Color32::TRANSPARENT, p.text, Stroke::new(1.0, p.border))
    };
    ui.add(
        Button::new(RichText::new(text).color(color))
            .fill(fill)
            .stroke(stroke)
            .corner_radius(255)
            .min_size(vec2(0.0, 28.0)),
    )
}

pub fn toggle(ui: &mut Ui, p: &Palette, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let track = lerp_color(p.selected, p.text, t);
    let knob = lerp_color(p.muted, p.on_text, t);
    let painter = ui.painter();
    painter.rect(rect, 10.0, track, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, t);
    painter.circle_filled(egui::pos2(x, rect.center().y), 7.0, knob);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A hairline progress bar with the percentage beside it.
pub fn progress(ui: &mut Ui, p: &Palette, fraction: f32) {
    ui.label(RichText::new(format!("{:.0}%", fraction * 100.0)).color(p.muted));
    let (rect, _) = ui.allocate_exact_size(vec2(56.0, 4.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, p.selected);
    let mut done = rect;
    done.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    painter.rect_filled(done, 2.0, p.text);
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}
