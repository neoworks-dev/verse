//! A layer surface drawn with egui: egui lays it out and `raster` paints it on the CPU into a
//! shared-memory buffer, so there is no window, toolkit or GPU context. The menu and the live
//! transcript bubble above the pill are both one of these.

mod raster;
pub mod style;

use std::time::{Duration, Instant};

use anyhow::Result;
use egui::{Event, Pos2, RawInput, Rect, Ui, ViewportId, vec2};
use smithay_client_toolkit::{
    compositor::{CompositorState, Region},
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface},
    },
    shm::{Shm, slot::SlotPool},
};
use wayland_client::{QueueHandle, protocol::{wl_shm, wl_surface::WlSurface}};

use crate::{config::Theme, overlay::State};

pub struct Options {
    pub namespace: &'static str,
    pub width: u32,
    /// The tallest it gets; content that needs less makes it shorter.
    pub max_height: u32,
    /// Distance from the screen's bottom edge.
    pub bottom: i32,
    pub keyboard: KeyboardInteractivity,
    /// Let clicks fall through to whatever is below.
    pub click_through: bool,
}

pub struct Canvas {
    layer: LayerSurface,
    pool: SlotPool,
    ctx: egui::Context,
    raster: raster::Raster,
    width: u32,
    max_height: u32,
    /// The surface's current height, in logical pixels.
    height: u32,
    scale: i32,
    configured: bool,
    /// When egui next wants to be drawn, if at all.
    next_paint: Option<Instant>,
    started: Instant,
    events: Vec<Event>,
    pub focused: bool,
}

impl Canvas {
    pub fn new(
        compositor: &CompositorState,
        layer_shell: &LayerShell,
        shm: &Shm,
        qh: &QueueHandle<State>,
        theme: Theme,
        options: Options,
    ) -> Result<Self> {
        let surface = compositor.create_surface(qh);
        if options.click_through {
            let nothing = Region::new(compositor)?;
            surface.set_input_region(Some(nothing.wl_region()));
        }
        let layer = layer_shell.create_layer_surface(qh, surface, Layer::Overlay, Some(options.namespace), None);
        layer.set_anchor(Anchor::BOTTOM);
        layer.set_margin(0, 0, options.bottom, 0);
        layer.set_size(options.width, options.max_height);
        layer.set_keyboard_interactivity(options.keyboard);
        layer.commit();

        let ctx = egui::Context::default();
        style::install_fonts(&ctx);
        style::apply(&ctx, theme);
        Ok(Self {
            layer,
            pool: SlotPool::new((options.width * options.max_height * 4) as usize, shm)?,
            ctx,
            raster: raster::Raster::default(),
            width: options.width,
            max_height: options.max_height,
            height: options.max_height,
            scale: 1,
            configured: false,
            next_paint: Some(Instant::now()),
            started: Instant::now(),
            events: Vec::new(),
            focused: false,
        })
    }

    pub fn surface(&self) -> &WlSurface {
        self.layer.wl_surface()
    }

    pub fn is_layer(&self, layer: &LayerSurface) -> bool {
        &self.layer == layer
    }

    pub fn configure(&mut self) {
        self.configured = true;
        self.wake();
    }

    pub fn set_scale(&mut self, scale: i32) {
        self.scale = scale.max(1);
        self.wake();
    }

    /// Draw again at the next tick.
    pub fn wake(&mut self) {
        self.next_paint = Some(Instant::now());
    }

    pub fn push(&mut self, event: Event) {
        self.events.push(event);
        self.wake();
    }

    /// Runs egui and draws a frame, if one is due. `ui` returns the height its content took;
    /// the surface is sized to it.
    pub fn tick(&mut self, ui: impl FnMut(&mut Ui) -> f32) -> Option<egui::PlatformOutput> {
        if !self.configured || self.next_paint.is_none_or(|at| Instant::now() < at) {
            return None;
        }
        let mut ui = ui;
        let ppp = self.scale as f32;
        let mut input = RawInput {
            // Laid out at full height so the content's natural size doesn't depend on the current one.
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(self.width as f32, self.max_height as f32))),
            time: Some(self.started.elapsed().as_secs_f64()),
            events: std::mem::take(&mut self.events),
            focused: self.focused,
            ..RawInput::default()
        };
        input.viewports.entry(ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);

        let mut content = 0.0;
        let output = self.ctx.run_ui(input, |egui_ui| content = ui(egui_ui));
        self.next_paint = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map(|v| v.repaint_delay)
            .filter(|d| *d < Duration::from_secs(3600))
            .map(|d| Instant::now() + d);

        self.raster.set_textures(&output.textures_delta);
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);

        // Fit the surface to the content. Content is top-aligned, so drawing only the top
        // `height` rows of the layout is exact; the new size applies after the compositor's
        // next configure, which also brings the next paint.
        let height = (content.ceil() as u32).clamp(1, self.max_height);
        if height != self.height {
            self.height = height;
            self.layer.set_size(self.width, height);
            self.layer.commit();
            self.configured = false;
            self.raster.free_textures(&output.textures_delta);
            return Some(output.platform_output);
        }
        let (w, h) = (self.width as i32 * self.scale, self.height as i32 * self.scale);
        if let Ok((buffer, canvas)) = self.pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) {
            self.raster.paint(&primitives, output.pixels_per_point, canvas, w as usize, h as usize);
            let surface = self.layer.wl_surface();
            surface.set_buffer_scale(self.scale);
            surface.damage_buffer(0, 0, w, h);
            if buffer.attach_to(surface).is_ok() {
                self.layer.commit();
            }
        }
        self.raster.free_textures(&output.textures_delta);
        Some(output.platform_output)
    }
}
