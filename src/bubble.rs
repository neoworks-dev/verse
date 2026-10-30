//! The live transcript above the pill while you talk. Clicks go through it, and it never takes
//! the keyboard, so it can't get in the way of the window you are dictating into.

use anyhow::Result;
use egui::{Frame, RichText, Stroke, Ui, UiBuilder};
use smithay_client_toolkit::{
    compositor::CompositorState,
    shell::wlr_layer::{KeyboardInteractivity, LayerShell, LayerSurface},
    shm::Shm,
};
use wayland_client::{QueueHandle, protocol::wl_surface::WlSurface};

use crate::{
    canvas::{Canvas, Options, style::Palette},
    config::Theme,
    overlay::State,
};

const WIDTH: u32 = 420;
/// Roughly three lines at this width; older text scrolls off the top.
const MAX_CHARS: usize = 160;

pub struct Bubble {
    canvas: Canvas,
    palette: Palette,
    text: String,
}

impl Bubble {
    pub fn new(
        compositor: &CompositorState,
        layer_shell: &LayerShell,
        shm: &Shm,
        qh: &QueueHandle<State>,
        bottom: i32,
        theme: Theme,
    ) -> Result<Self> {
        let options = Options {
            namespace: "verse-transcript",
            width: WIDTH,
            max_height: 120,
            bottom,
            keyboard: KeyboardInteractivity::None,
            click_through: true,
        };
        let canvas = Canvas::new(compositor, layer_shell, shm, qh, theme, options)?;
        Ok(Self { canvas, palette: Palette::of(theme), text: String::new() })
    }

    pub fn surface(&self) -> &WlSurface {
        self.canvas.surface()
    }

    pub fn is_layer(&self, layer: &LayerSurface) -> bool {
        self.canvas.is_layer(layer)
    }

    pub fn configure(&mut self) {
        self.canvas.configure();
    }

    pub fn set_scale(&mut self, scale: i32) {
        self.canvas.set_scale(scale);
    }

    pub fn set_text(&mut self, text: &str) {
        let text = tail(text.trim(), MAX_CHARS);
        if text != self.text {
            self.text = text;
            self.canvas.wake();
        }
    }

    pub fn tick(&mut self) {
        let (p, text) = (self.palette, &self.text);
        self.canvas.tick(|ui| show(ui, &p, text));
    }
}

fn show(ui: &mut Ui, p: &Palette, text: &str) -> f32 {
    // Hug the text: a short phrase makes a small bubble, centred over the pill.
    let galley = ui.painter().layout(text.to_string(), egui::FontId::proportional(14.0), p.text, WIDTH as f32 - 34.0);
    let width = galley.size().x + 32.0;
    let left = (WIDTH as f32 - width) / 2.0;
    let rect = egui::Rect::from_min_size(egui::pos2(left, 0.5), egui::vec2(width, ui.max_rect().height() - 1.0));
    let mut height = 0.0;
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        let shown = Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(14.0)
            .inner_margin(egui::Margin::symmetric(16, 10))
            .show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(text).size(14.0).color(p.text)).wrap());
            });
        height = shown.response.rect.height() + 1.0;
    });
    height
}

/// The end of `text`, cut at a word boundary and marked with an ellipsis when cut.
fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let cut: String = text.chars().skip(count - max).collect();
    let cut = cut.split_once(' ').map_or(cut.as_str(), |(_, rest)| rest);
    format!("…{cut}")
}
