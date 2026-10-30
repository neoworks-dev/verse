//! The settings dropdown that opens above the pill, drawn on a `Canvas` like the transcript
//! bubble.

mod ui;

use std::process::{Command, Stdio};

use anyhow::Result;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use smithay_client_toolkit::{
    compositor::CompositorState,
    seat::{
        keyboard::{KeyEvent, Keysym, Modifiers as WlModifiers},
        pointer::PointerEventKind,
    },
    shell::wlr_layer::{KeyboardInteractivity, LayerShell, LayerSurface},
    shm::Shm,
};
use wayland_client::{QueueHandle, protocol::wl_surface::WlSurface};

pub use ui::Page;

use crate::{
    canvas::{Canvas, Options},
    overlay::State,
};

// Linux input event codes for mouse buttons.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

pub struct Menu {
    canvas: Canvas,
    settings: ui::Settings,
    modifiers: Modifiers,
    had_focus: bool,
}

impl Menu {
    /// `bottom` is the distance from the screen's bottom edge, so it sits just above the pill.
    pub fn new(
        compositor: &CompositorState,
        layer_shell: &LayerShell,
        shm: &Shm,
        qh: &QueueHandle<State>,
        bottom: i32,
        page: Page,
    ) -> Result<Self> {
        let settings = ui::Settings::new(page);
        let options = Options {
            namespace: "verse-menu",
            width: 320,
            max_height: 460,
            bottom,
            // Typing into the vocabulary needs the keyboard; on demand gives it up when you click away.
            keyboard: KeyboardInteractivity::OnDemand,
            click_through: false,
        };
        let canvas = Canvas::new(compositor, layer_shell, shm, qh, settings.theme(), options)?;
        Ok(Self { canvas, settings, modifiers: Modifiers::default(), had_focus: false })
    }

    pub fn surface(&self) -> &WlSurface {
        self.canvas.surface()
    }

    pub fn is_layer(&self, layer: &LayerSurface) -> bool {
        self.canvas.is_layer(layer)
    }

    /// The user closed it, or clicked somewhere else after using it.
    pub fn closed(&self) -> bool {
        self.settings.closed || (self.had_focus && !self.canvas.focused)
    }

    pub fn theme(&self) -> crate::config::Theme {
        self.settings.theme()
    }

    pub fn configure(&mut self) {
        self.canvas.configure();
    }

    pub fn set_scale(&mut self, scale: i32) {
        self.canvas.set_scale(scale);
    }

    pub fn pointer(&mut self, kind: &PointerEventKind, (x, y): (f64, f64)) {
        let pos = Pos2::new(x as f32, y as f32);
        let modifiers = self.modifiers;
        match kind {
            PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                self.canvas.push(Event::PointerMoved(pos));
            }
            PointerEventKind::Leave { .. } => self.canvas.push(Event::PointerGone),
            PointerEventKind::Press { button, .. } | PointerEventKind::Release { button, .. } => {
                let button = match *button {
                    BTN_LEFT => PointerButton::Primary,
                    BTN_RIGHT => PointerButton::Secondary,
                    BTN_MIDDLE => PointerButton::Middle,
                    _ => return,
                };
                let pressed = matches!(kind, PointerEventKind::Press { .. });
                self.canvas.push(Event::PointerButton { pos, button, pressed, modifiers });
            }
            PointerEventKind::Axis { horizontal, vertical, .. } => {
                // Wayland scrolls down with positive values; egui moves content down with them.
                let delta = vec2(-horizontal.absolute as f32, -vertical.absolute as f32);
                self.canvas.push(Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers,
                });
            }
        }
    }

    pub fn focus(&mut self, focused: bool) {
        self.canvas.focused = focused;
        self.had_focus |= focused;
        self.canvas.push(Event::WindowFocused(focused));
    }

    pub fn modifiers(&mut self, m: &WlModifiers) {
        self.modifiers = Modifiers { alt: m.alt, ctrl: m.ctrl, shift: m.shift, mac_cmd: false, command: m.ctrl };
        self.canvas.push(Event::ModifiersChanged(self.modifiers));
    }

    pub fn key(&mut self, event: &KeyEvent, pressed: bool, repeat: bool) {
        let modifiers = self.modifiers;
        let key = key_of(event);
        if let Some(key) = key {
            if pressed && modifiers.ctrl {
                match key {
                    Key::C => self.canvas.push(Event::Copy),
                    Key::X => self.canvas.push(Event::Cut),
                    Key::V => {
                        if let Some(text) = paste() {
                            self.canvas.push(Event::Paste(text));
                        }
                    }
                    _ => {}
                }
            }
            self.canvas.push(Event::Key { key, physical_key: None, pressed, repeat, modifiers });
        }
        if pressed && !modifiers.ctrl && !modifiers.alt {
            if let Some(text) = event.utf8.as_ref().filter(|t| !t.chars().any(char::is_control)) {
                self.canvas.push(Event::Text(text.clone()));
            }
        }
    }

    /// Draws a frame if egui asked for one by now.
    pub fn tick(&mut self) {
        let settings = &mut self.settings;
        let Some(output) = self.canvas.tick(|ui| settings.ui(ui)) else { return };
        for command in &output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                copy(text);
            }
        }
    }
}

fn key_of(event: &KeyEvent) -> Option<Key> {
    Some(match event.keysym {
        Keysym::BackSpace => Key::Backspace,
        Keysym::Delete => Key::Delete,
        Keysym::Return | Keysym::KP_Enter => Key::Enter,
        Keysym::Escape => Key::Escape,
        Keysym::Tab | Keysym::ISO_Left_Tab => Key::Tab,
        Keysym::Left => Key::ArrowLeft,
        Keysym::Right => Key::ArrowRight,
        Keysym::Up => Key::ArrowUp,
        Keysym::Down => Key::ArrowDown,
        Keysym::Home => Key::Home,
        Keysym::End => Key::End,
        Keysym::Page_Up => Key::PageUp,
        Keysym::Page_Down => Key::PageDown,
        _ => {
            // Letters, for shortcuts like Ctrl+A and Ctrl+Z.
            let c = event.utf8.as_deref()?.chars().next()?;
            return c.is_ascii_alphabetic().then(|| Key::from_name(&c.to_ascii_uppercase().to_string())).flatten();
        }
    })
}

fn paste() -> Option<String> {
    let out = Command::new("wl-paste").arg("--no-newline").stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn copy(text: &str) {
    use std::io::Write;
    if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).stderr(Stdio::null()).spawn() {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
    }
}
