//! The pill on screen: a wlr-layer-shell surface drawn in software into a shared-memory buffer.
//! No toolkit and no GPU context, so it costs a couple of megabytes while visible. Its button
//! opens the settings menu right above it.

use std::{
    collections::VecDeque,
    os::fd::AsRawFd,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_dispatch2, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use tiny_skia::{Color, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};
use wayland_client::{
    Connection, EventQueue, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};

use crate::{
    bubble::Bubble,
    config::Theme,
    menu::{Menu, Page},
};

const WIDTH: u32 = 168;
const HEIGHT: u32 = 52;
const BOTTOM_MARGIN: i32 = 48;
/// Space between the pill and the menu above it.
const MENU_GAP: i32 = 10;
const BARS: usize = 13;
/// Each bar is this much audio, so the pill shows about the last second.
const BAR_INTERVAL: Duration = Duration::from_millis(80);
/// The quietest peak the bars are scaled to (RMS), and how many decibels below the peak a bar
/// shrinks to nothing.
const PEAK_FLOOR: f32 = 0.03;
const RANGE_DB: f32 = 24.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Recording,
    Processing,
    /// The menu is open; the pill only anchors it.
    Menu,
}

pub struct Overlay {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
}

pub(crate) struct State {
    registry: RegistryState,
    seat: SeatState,
    output: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pixmap: Pixmap,
    menu: Option<Menu>,
    bubble: Option<Bubble>,

    scale: i32,
    configured: bool,
    /// Whether the pill needs drawing while nothing on it animates.
    dirty: bool,
    closed: bool,
    button_clicked: bool,

    theme: Theme,
    phase: Phase,
    /// Loudness of the last `BARS` slices of audio, oldest first.
    levels: VecDeque<f32>,
    /// The slice being recorded now: its summed power over `samples` readings, and when it started.
    power: f32,
    samples: u32,
    current_since: Instant,
    /// The loudest recent slice, which the bars are scaled to.
    peak: f32,
    frame: u32,
}

impl Overlay {
    pub fn new(theme: Theme) -> Result<Self> {
        let conn = Connection::connect_to_env().context("connecting to Wayland")?;
        let (globals, queue) = registry_queue_init(&conn)?;
        let qh = queue.handle();

        let compositor = CompositorState::bind(&globals, &qh).context("no wl_compositor")?;
        let layer_shell = LayerShell::bind(&globals, &qh).context("compositor lacks wlr-layer-shell")?;
        let shm = Shm::bind(&globals, &qh).context("no wl_shm")?;

        let surface = compositor.create_surface(&qh);
        let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("verse"), None);
        layer.set_anchor(Anchor::BOTTOM);
        layer.set_margin(0, 0, BOTTOM_MARGIN, 0);
        layer.set_size(WIDTH, HEIGHT);
        // Never take keyboard focus: the text has to land in the window the user was typing in.
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();

        let pool = SlotPool::new((WIDTH * HEIGHT * 4) as usize, &shm)?;
        let state = State {
            registry: RegistryState::new(&globals),
            seat: SeatState::new(&globals, &qh),
            output: OutputState::new(&globals, &qh),
            compositor,
            layer_shell,
            shm,
            pool,
            layer,
            pointer: None,
            keyboard: None,
            pixmap: Pixmap::new(WIDTH, HEIGHT).unwrap(),
            menu: None,
            bubble: None,
            scale: 1,
            configured: false,
            dirty: true,
            closed: false,
            button_clicked: false,
            theme,
            phase: Phase::Recording,
            levels: VecDeque::from(vec![0.0; BARS]),
            power: 0.0,
            samples: 0,
            current_since: Instant::now(),
            peak: PEAK_FLOOR,
            frame: 0,
        };
        Ok(Self { conn, queue, state })
    }

    /// Handles Wayland events for up to `timeout`, then draws what changed.
    pub fn pump(&mut self, timeout: Duration) -> Result<()> {
        self.queue.flush()?;
        self.queue.dispatch_pending(&mut self.state)?;
        if let Some(guard) = self.queue.prepare_read() {
            let mut fd = libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut fd, 1, timeout.as_millis() as i32) };
            if ready > 0 {
                // WouldBlock just means another reader got there first.
                let _ = guard.read();
            }
        }
        self.queue.dispatch_pending(&mut self.state)?;

        let state = &mut self.state;
        if let Some(bubble) = &mut state.bubble {
            bubble.tick();
        }
        if let Some(menu) = &mut state.menu {
            menu.tick();
            // The theme can be switched in the menu; the pill follows.
            if menu.theme() != state.theme {
                state.theme = menu.theme();
                state.dirty = true;
            }
        }
        if state.configured && (state.dirty || state.phase != Phase::Menu) {
            state.draw();
        }
        Ok(())
    }

    pub fn push_level(&mut self, rms: f32) {
        let state = &mut self.state;
        state.power += rms * rms;
        state.samples += 1;
        // Once a slice is over it becomes a bar on the right and the rest move left.
        while state.current_since.elapsed() >= BAR_INTERVAL {
            let level = state.slice_level();
            state.levels.pop_front();
            state.levels.push_back(level);
            state.power = 0.0;
            state.samples = 0;
            state.current_since += BAR_INTERVAL;
        }
    }

    pub fn set_phase(&mut self, phase: Phase) {
        self.state.phase = phase;
        self.state.dirty = true;
    }

    /// Whether the button on the left was clicked since the last call.
    pub fn take_button_click(&mut self) -> bool {
        std::mem::take(&mut self.state.button_clicked)
    }

    pub fn closed(&self) -> bool {
        self.state.closed
    }

    /// Shows `text` in the bubble above the pill, opening it on the first words.
    pub fn set_transcript(&mut self, text: &str) -> Result<()> {
        let state = &mut self.state;
        if state.bubble.is_none() {
            if text.trim().is_empty() {
                return Ok(());
            }
            let qh = self.queue.handle();
            let bottom = BOTTOM_MARGIN + HEIGHT as i32 + MENU_GAP;
            state.bubble = Some(Bubble::new(&state.compositor, &state.layer_shell, &state.shm, &qh, bottom, state.theme)?);
        }
        state.bubble.as_mut().unwrap().set_text(text);
        Ok(())
    }

    /// Opens the settings menu above the pill.
    pub fn open_menu(&mut self, page: Page) -> Result<()> {
        let qh = self.queue.handle();
        let state = &mut self.state;
        let bottom = BOTTOM_MARGIN + HEIGHT as i32 + MENU_GAP;
        // The menu takes the bubble's place.
        state.bubble = None;
        state.menu = Some(Menu::new(&state.compositor, &state.layer_shell, &state.shm, &qh, bottom, page)?);
        state.levels.iter_mut().for_each(|l| *l = 0.0);
        self.set_phase(Phase::Menu);
        Ok(())
    }

    pub fn menu_closed(&self) -> bool {
        self.state.menu.as_ref().is_none_or(Menu::closed)
    }

    /// Unmaps the pill (and menu) and waits until the compositor has processed it.
    pub fn hide(mut self) {
        self.state.menu = None;
        self.state.bubble = None;
        self.state.layer.wl_surface().attach(None, 0, 0);
        self.state.layer.commit();
        let _ = self.queue.roundtrip(&mut self.state);
        drop(self.state);
        let _ = self.conn.flush();
    }
}

struct Palette {
    background: Color,
    border: Color,
    foreground: Color,
}

impl Palette {
    fn of(theme: Theme) -> Self {
        match theme {
            // The same palette as the menu and neoworks.dev.
            Theme::Dark => Self {
                background: Color::from_rgba8(16, 16, 16, 242),
                border: Color::from_rgba8(44, 44, 44, 255),
                foreground: Color::from_rgba8(242, 240, 237, 255),
            },
            Theme::Light => Self {
                background: Color::from_rgba8(255, 255, 255, 245),
                border: Color::from_rgba8(229, 227, 223, 255),
                foreground: Color::from_rgba8(20, 20, 20, 255),
            },
        }
    }
}

impl State {
    fn draw(&mut self) {
        let scale = self.scale.max(1);
        let (w, h) = (WIDTH * scale as u32, HEIGHT * scale as u32);
        if self.pixmap.width() != w || self.pixmap.height() != h {
            self.pixmap = Pixmap::new(w, h).unwrap();
        }
        self.render(scale as f32);

        let stride = w as i32 * 4;
        let Ok((buffer, canvas)) =
            self.pool.create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
        else {
            return;
        };
        // tiny-skia gives premultiplied RGBA; wl_shm's ARGB8888 is BGRA in memory.
        for (dst, src) in canvas.chunks_exact_mut(4).zip(self.pixmap.data().chunks_exact(4)) {
            dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
        }

        let surface = self.layer.wl_surface();
        surface.set_buffer_scale(scale);
        surface.damage_buffer(0, 0, w as i32, h as i32);
        if buffer.attach_to(surface).is_ok() {
            self.layer.commit();
        }
        self.frame = self.frame.wrapping_add(1);
        self.dirty = false;
    }

    fn render(&mut self, s: f32) {
        let palette = Palette::of(self.theme);
        let (w, h) = (WIDTH as f32, HEIGHT as f32);
        let t = Transform::from_scale(s, s);
        let pixmap = &mut self.pixmap;
        pixmap.fill(Color::TRANSPARENT);

        let mut paint = Paint { anti_alias: true, ..Paint::default() };
        let line = |x0: f32, y0: f32, x1: f32, y1: f32| {
            let mut pb = PathBuilder::new();
            pb.move_to(x0, y0);
            pb.line_to(x1, y1);
            pb.finish().unwrap()
        };
        let round = |width: f32| Stroke { width, line_cap: LineCap::Round, ..Stroke::default() };

        // The pill: a fat round-capped line, border first and background 1px inside it.
        let body = line(h / 2.0, h / 2.0, w - h / 2.0, h / 2.0);
        paint.set_color(palette.border);
        pixmap.stroke_path(&body, &paint, &round(h), t, None);
        paint.set_color(palette.background);
        pixmap.stroke_path(&body, &paint, &round(h - 2.0), t, None);

        // The button: a ring with two bars in it, filled in while the menu is open.
        let (cx, cy, r) = (h / 2.0, h / 2.0, 13.0);
        let ring = PathBuilder::from_circle(cx, cy, r).unwrap();
        paint.set_color(palette.foreground);
        if self.phase == Phase::Menu {
            pixmap.fill_path(&ring, &paint, tiny_skia::FillRule::Winding, t, None);
            paint.set_color(palette.background);
        } else {
            pixmap.stroke_path(&ring, &paint, &Stroke { width: 2.0, ..Stroke::default() }, t, None);
        }
        for dx in [-3.0, 3.0] {
            pixmap.stroke_path(&line(cx + dx, cy - 5.0, cx + dx, cy + 5.0), &paint, &round(2.6), t, None);
        }

        // The waveform.
        let left = h + 2.0;
        let right = w - 18.0;
        let gap = (right - left) / (BARS - 1) as f32;
        let max_half = h / 2.0 - 12.0;
        let mut bars = palette.foreground;
        if self.phase != Phase::Recording {
            bars.set_alpha(0.45);
        }
        paint.set_color(bars);
        let line_at = |pixmap: &mut Pixmap, paint: &Paint, x: f32, level: f32| {
            let half = 2.0 + level * max_half;
            pixmap.stroke_path(&line(x, cy - half, x, cy + half), paint, &round(2.6), t, None);
        };
        match self.phase {
            // A slow travelling wave while whisper works.
            Phase::Processing => {
                for i in 0..BARS {
                    let phase = self.frame as f32 * 0.18 - i as f32 * 0.55;
                    line_at(pixmap, &paint, left + i as f32 * gap, 0.15 + 0.35 * (0.5 + 0.5 * phase.sin()));
                }
            }
            Phase::Menu => {
                for i in 0..BARS {
                    line_at(pixmap, &paint, left + i as f32 * gap, 0.0);
                }
            }
            // The audio flows left: bars slide one gap per slice, the oldest fading out on the
            // left while the slice being recorded fades in on the right, already as loud as it is.
            Phase::Recording => {
                let shift = (self.current_since.elapsed().as_secs_f32() / BAR_INTERVAL.as_secs_f32()).min(1.0);
                let current = (self.power / self.samples.max(1) as f32).sqrt();
                let current = (1.0 + 20.0 * (current.max(1e-6) / self.peak).log10() / RANGE_DB).clamp(0.0, 1.0);
                let levels = self.levels.iter().copied().chain([current]);
                for (i, level) in levels.enumerate() {
                    let x = left + (i as f32 - shift) * gap;
                    let outside = (left - x).max(x - right).max(0.0);
                    let mut color = bars;
                    color.set_alpha(bars.alpha() * (1.0 - outside / gap).clamp(0.0, 1.0));
                    paint.set_color(color);
                    line_at(pixmap, &paint, x, level);
                }
            }
        }
    }

    /// The current slice's height: its loudness in decibels below the recent peak. The peak
    /// fades over a few seconds, so the bars use their height whatever the mic's gain is.
    fn slice_level(&mut self) -> f32 {
        let rms = (self.power / self.samples.max(1) as f32).sqrt();
        self.peak = (self.peak * 0.97).max(rms).max(PEAK_FLOOR);
        let db = 20.0 * (rms.max(1e-6) / self.peak).log10();
        (1.0 + db / RANGE_DB).clamp(0.0, 1.0)
    }

    fn menu_for(&mut self, surface: &wl_surface::WlSurface) -> Option<&mut Menu> {
        self.menu.as_mut().filter(|m| m.surface() == surface)
    }

    fn bubble_for(&mut self, surface: &wl_surface::WlSurface) -> Option<&mut Bubble> {
        self.bubble.as_mut().filter(|b| b.surface() == surface)
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        if let Some(menu) = self.menu_for(surface) {
            menu.set_scale(factor);
        } else if let Some(bubble) = self.bubble_for(surface) {
            bubble.set_scale(factor);
        } else {
            self.scale = factor;
            self.dirty = true;
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.closed = true;
    }
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, _: LayerSurfaceConfigure, _: u32) {
        if let Some(bubble) = self.bubble.as_mut().filter(|b| b.is_layer(layer)) {
            bubble.configure();
            return;
        }
        match &mut self.menu {
            Some(menu) if menu.is_layer(layer) => menu.configure(),
            _ => {
                self.configured = true;
                self.dirty = true;
            }
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat.get_pointer(qh, &seat).ok();
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat.get_keyboard(qh, &seat, None).ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for State {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            if let Some(menu) = self.menu_for(&event.surface) {
                menu.pointer(&event.kind, event.position);
                continue;
            }
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            if let PointerEventKind::Release { .. } = event.kind {
                let (x, y) = event.position;
                let r = HEIGHT as f64 / 2.0;
                if (x - r).hypot(y - r) <= r {
                    self.button_clicked = true;
                }
            }
        }
    }
}

// The pill never has keyboard focus, so all of this is the menu's.
impl KeyboardHandler for State {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, surface: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {
        if let Some(menu) = self.menu_for(surface) {
            menu.focus(true);
        }
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, surface: &wl_surface::WlSurface, _: u32) {
        if let Some(menu) = self.menu_for(surface) {
            menu.focus(false);
        }
    }
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        if let Some(menu) = &mut self.menu {
            menu.key(&event, true, false);
        }
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        if let Some(menu) = &mut self.menu {
            menu.key(&event, true, true);
        }
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        if let Some(menu) = &mut self.menu {
            menu.key(&event, false, false);
        }
    }
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: RawModifiers, _: u32) {
        if let Some(menu) = &mut self.menu {
            menu.modifiers(&modifiers);
        }
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(State);
delegate_dispatch2!(State);

