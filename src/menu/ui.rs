//! The menu's pages: a compact list like the account menu on neoworks.dev, where each entry
//! either changes a setting in place or opens a page of its own.

use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    thread,
    time::Duration,
};

use egui::{Align, Align2, FontFamily, FontId, Layout, RichText, Sense, TextEdit, Ui, UiBuilder, vec2};

use crate::canvas::style::{self, Palette, chip, divider, ghost, medium, nav_row, page_header, primary, row, toggle, value_row};
use crate::{
    config::{Config, Theme},
    models::{self, CATALOG, Model, Progress},
    presets::PRESETS,
    vocab::Replacement,
};

const LANGUAGES: &[(&str, &str)] = &[
    ("auto", "Detect automatically"),
    ("en", "English"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("nl", "Dutch"),
    ("pt", "Portuguese"),
    ("pl", "Polish"),
    ("ru", "Russian"),
    ("uk", "Ukrainian"),
    ("tr", "Turkish"),
    ("ja", "Japanese"),
    ("zh", "Chinese"),
    ("ko", "Korean"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Models,
    Language,
    Vocabulary,
    Replacements,
}

struct Download {
    model: &'static str,
    progress: Arc<Progress>,
    /// None while running, then the outcome.
    result: Arc<Mutex<Option<Result<(), String>>>>,
}

pub struct Settings {
    config: Config,
    palette: Palette,
    page: Page,
    download: Option<Download>,
    error: Option<String>,
    /// The vocabulary as typed, one term per line, so editing doesn't reflow it.
    vocabulary: String,
    /// The config file didn't parse; saving would replace it with defaults.
    readonly: bool,
    /// Set when the user closes the menu.
    pub closed: bool,
    /// How tall the card came out, so the surface can be sized to it.
    pub height: f32,
}

impl Settings {
    pub fn new(page: Page) -> Self {
        let (config, load_error) = Config::load_checked();
        Self {
            palette: Palette::of(config.theme),
            vocabulary: config.vocabulary.join("\n"),
            config,
            page,
            download: None,
            readonly: load_error.is_some(),
            error: load_error.map(|e| format!("{e}. Changes won't be saved until it's fixed.")),
            closed: false,
            height: 0.0,
        }
    }

    pub fn theme(&self) -> Theme {
        self.config.theme
    }

    /// Draws the menu and returns how tall it came out.
    pub fn ui(&mut self, ui: &mut Ui) -> f32 {
        self.poll_download();
        if self.download.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.page == Page::Home {
                self.closed = true;
            } else {
                self.page = Page::Home;
            }
        }

        let p = self.palette;
        let card = egui::Frame::new()
            .fill(p.card)
            .stroke(egui::Stroke::new(1.0, p.border))
            .corner_radius(16.0)
            .inner_margin(8.0);
        // The card takes the height its page needs, up to the menu's maximum, and the surface
        // is then sized to it; long pages scroll inside.
        let rect = ui.max_rect().shrink(0.5);
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            let shown = card.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                if let Some(error) = self.error.clone() {
                    self.error_banner(ui, &error);
                }
                match self.page {
                    Page::Home => self.home(ui),
                    Page::Models => self.models(ui),
                    Page::Language => self.language(ui),
                    Page::Vocabulary => self.vocabulary(ui),
                    Page::Replacements => self.replacements(ui),
                }
            });
            self.height = shown.response.rect.height() + 1.0;
        });
        self.height
    }

    fn save(&mut self) {
        if self.readonly {
            return;
        }
        if let Err(err) = self.config.save() {
            self.error = Some(format!("{err:#}"));
        }
    }

    fn error_banner(&mut self, ui: &mut Ui, error: &str) {
        let p = self.palette;
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            let text = egui::Label::new(RichText::new(error).color(p.error).size(12.5)).wrap();
            ui.allocate_ui(vec2(ui.available_width() - 34.0, 0.0), |ui| ui.add(text));
            let (rect, dismiss) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
            if dismiss.hovered() {
                ui.painter().rect_filled(rect, 8.0, p.hover);
            }
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "×", FontId::new(16.0, FontFamily::Proportional), p.muted);
            if dismiss.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.error = None;
            }
        });
        ui.add_space(4.0);
        divider(ui, &p);
    }

    fn home(&mut self, ui: &mut Ui) {
        let p = self.palette;

        // Title and close, like the account card at the top of the site's menu.
        ui.horizontal(|ui| {
            ui.set_min_height(48.0);
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("Verse").size(16.0).family(medium()).color(p.text));
                ui.label(RichText::new("Local speech to text").size(12.5).color(p.muted));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(4.0);
                let (rect, close) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                if close.hovered() {
                    ui.painter().rect_filled(rect, 8.0, p.hover);
                }
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, "×", FontId::new(18.0, FontFamily::Proportional), p.muted);
                if close.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.closed = true;
                }
            });
        });
        ui.add_space(4.0);
        divider(ui, &p);
        ui.add_space(4.0);

        let model = if models::is_downloaded(&self.config.model) { self.config.model.as_str() } else { "None" };
        if nav_row(ui, &p, "Model", model).clicked() {
            self.page = Page::Models;
        }
        let language = LANGUAGES
            .iter()
            .find(|(code, _)| *code == self.config.language)
            .map_or(self.config.language.as_str(), |(_, name)| name);
        if nav_row(ui, &p, "Language", language).clicked() {
            self.page = Page::Language;
        }
        let vocabulary = vocabulary_summary(&self.config);
        if nav_row(ui, &p, "Vocabulary", &vocabulary).clicked() {
            self.page = Page::Vocabulary;
        }
        let replacements = match self.config.replacements.len() {
            0 => "None".to_string(),
            1 => "1 rule".to_string(),
            n => format!("{n} rules"),
        };
        if nav_row(ui, &p, "Replacements", &replacements).clicked() {
            self.page = Page::Replacements;
        }

        ui.add_space(4.0);
        divider(ui, &p);
        ui.add_space(4.0);

        let mut changed = false;
        // Like the site's "Appearance  Dark": the row itself switches the theme.
        let theme = match self.config.theme {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
        };
        if value_row(ui, &p, "Appearance", theme).clicked() {
            self.config.theme = match self.config.theme {
                Theme::Dark => Theme::Light,
                Theme::Light => Theme::Dark,
            };
            style::apply(ui.ctx(), self.config.theme);
            self.palette = Palette::of(self.config.theme);
            changed = true;
        }
        row(ui, &p, "Live transcript", |ui| {
            changed |= toggle(ui, &p, &mut self.config.live_preview).changed();
        });
        row(ui, &p, "Use the GPU", |ui| {
            changed |= toggle(ui, &p, &mut self.config.use_gpu).changed();
        });
        row(ui, &p, "Longest recording", |ui| {
            ui.label(RichText::new(format!("{} s", self.config.max_seconds)).color(p.muted));
            let slider = egui::Slider::new(&mut self.config.max_seconds, 10..=600)
                .show_value(false)
                .trailing_fill(true)
                .handle_shape(egui::style::HandleShape::Circle);
            ui.scope(|ui| {
                ui.spacing_mut().slider_width = 90.0;
                // The filled part of the rail; scoped because it is also the text selection colour.
                ui.visuals_mut().selection.bg_fill = p.text;
                changed |= ui.add(slider).changed();
            });
        });
        if changed {
            self.save();
        }
    }

    fn models(&mut self, ui: &mut Ui) {
        let p = self.palette;
        if page_header(ui, &p, "Model") {
            self.page = Page::Home;
        }
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.add(egui::Label::new(
                RichText::new("Bigger models are more accurate, but slower and use more memory.").size(12.5).color(p.muted),
            ).wrap());
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            for model in CATALOG {
                self.model_row(ui, model);
            }
        });
    }

    fn model_row(&mut self, ui: &mut Ui, model: &'static Model) {
        let p = self.palette;
        let downloaded = models::is_downloaded(model.name);
        let selected = self.config.model == model.name;

        // The row is allocated before its contents, so the buttons inside sit on top of it
        // and get their own clicks.
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::click());
        let clickable = downloaded && !selected;
        let fill = if selected {
            Some(p.selected)
        } else if clickable && response.hovered() {
            Some(p.hover)
        } else {
            None
        };
        if let Some(fill) = fill {
            ui.painter().rect_filled(rect, 10.0, fill);
        }
        if clickable {
            if response.clicked() {
                self.config.model = model.name.to_string();
                self.save();
            }
            response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }

        let inner = rect.shrink2(vec2(10.0, 6.0));
        let mut ui = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(3.0);
            ui.label(RichText::new(model.name).family(medium()).color(p.text));
            ui.label(RichText::new(format!("{} MB · {}", model.size_mb, model.description)).size(12.5).color(p.muted));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            self.model_action(ui, model.name, downloaded, selected);
        });
    }

    fn model_action(&mut self, ui: &mut Ui, name: &'static str, downloaded: bool, selected: bool) {
        let p = self.palette;
        if let Some(download) = &self.download {
            if download.model == name {
                if ghost(ui, &p, "Cancel").clicked() {
                    download.progress.cancel.store(true, Ordering::Relaxed);
                }
                let done = download.progress.done.load(Ordering::Relaxed) as f32;
                let total = download.progress.total.load(Ordering::Relaxed).max(1) as f32;
                style::progress(ui, &p, done / total);
                return;
            }
        }
        if selected && downloaded {
            ui.label(RichText::new("In use").color(p.muted));
        } else if downloaded {
            if ghost(ui, &p, "Remove").clicked() {
                if let Err(err) = models::delete(name) {
                    self.error = Some(format!("{err:#}"));
                }
            }
        } else {
            ui.add_enabled_ui(self.download.is_none(), |ui| {
                if primary(ui, &p, "Get").clicked() {
                    self.start_download(name);
                }
            });
        }
    }

    fn start_download(&mut self, model: &'static str) {
        let progress = Arc::new(Progress::default());
        let result = Arc::new(Mutex::new(None));
        thread::spawn({
            let progress = progress.clone();
            let result = result.clone();
            move || {
                let outcome = models::download(model, progress).map_err(|e| format!("{e:#}"));
                *result.lock().unwrap() = Some(outcome);
            }
        });
        self.download = Some(Download { model, progress, result });
    }

    fn poll_download(&mut self) {
        let Some(download) = &self.download else { return };
        let Some(outcome) = download.result.lock().unwrap().take() else { return };
        let model = download.model;
        self.download = None;
        match outcome {
            Ok(()) => {
                // A model you just downloaded is the one you want to use.
                self.config.model = model.to_string();
                self.save();
            }
            Err(err) if err == "cancelled" => {}
            Err(err) => self.error = Some(format!("Downloading {model} failed: {err}")),
        }
    }

    fn language(&mut self, ui: &mut Ui) {
        let p = self.palette;
        if page_header(ui, &p, "Language") {
            self.page = Page::Home;
        }
        ui.add_space(4.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (code, name) in LANGUAGES {
                let selected = self.config.language == *code;
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                if selected {
                    ui.painter().rect_filled(rect, 10.0, p.selected);
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 10.0, p.hover);
                }
                let y = rect.center().y;
                ui.painter().text(egui::pos2(rect.left() + 10.0, y), Align2::LEFT_CENTER, *name, FontId::new(14.0, FontFamily::Proportional), p.text);
                if selected {
                    ui.painter().text(egui::pos2(rect.right() - 12.0, y), Align2::RIGHT_CENTER, "✓", FontId::new(14.0, FontFamily::Proportional), p.text);
                }
                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !selected {
                    self.config.language = code.to_string();
                    self.save();
                }
            }
        });
    }

    fn vocabulary(&mut self, ui: &mut Ui) {
        let p = self.palette;
        if page_header(ui, &p, "Vocabulary") {
            self.page = Page::Home;
        }
        let mut changed = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let indent = |ui: &mut Ui, add: &mut dyn FnMut(&mut Ui)| {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(10.0);
                    ui.set_max_width(ui.available_width() - 10.0);
                    add(ui);
                });
            };
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(egui::Label::new(
                    RichText::new("Words whisper should expect. Words that sound like one are corrected to it.")
                        .size(12.5)
                        .color(p.muted),
                ).wrap());
            });
            ui.add_space(4.0);
            indent(ui, &mut |ui| {
                ui.label(RichText::new("Presets").size(12.5).family(medium()).color(p.text));
            });
            indent(ui, &mut |ui| {
                for preset in PRESETS {
                    let on = self.config.presets.iter().any(|id| id == preset.id);
                    let response = chip(ui, &p, preset.name, on).on_hover_text(preset.terms.join(", "));
                    if response.clicked() {
                        if on {
                            self.config.presets.retain(|id| id != preset.id);
                        } else {
                            self.config.presets.push(preset.id.to_string());
                        }
                        changed = true;
                    }
                }
            });
            ui.add_space(6.0);
            indent(ui, &mut |ui| {
                ui.label(RichText::new("Your words, one per line").size(12.5).family(medium()).color(p.text));
            });
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                let edit = TextEdit::multiline(&mut self.vocabulary)
                    .hint_text("neoworks\nuseEffect\nkubectl")
                    .desired_rows(6)
                    .margin(vec2(10.0, 8.0))
                    .desired_width(ui.available_width() - 10.0);
                if ui.add(edit).changed() {
                    self.config.vocabulary = self
                        .vocabulary
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .map(String::from)
                        .collect();
                    changed = true;
                }
            });
        });
        if changed {
            self.save();
        }
    }

    fn replacements(&mut self, ui: &mut Ui) {
        let p = self.palette;
        if page_header(ui, &p, "Replacements") {
            self.page = Page::Home;
        }
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.add(egui::Label::new(
                RichText::new("Fixed rewrites, applied last. Whole words, ignoring case.").size(12.5).color(p.muted),
            ).wrap());
        });
        ui.add_space(6.0);
        let mut changed = false;
        let mut remove = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, rule) in self.config.replacements.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    let width = ((ui.available_width() - 64.0) / 2.0).max(60.0);
                    let from = TextEdit::singleline(&mut rule.from).hint_text("heard").margin(vec2(8.0, 6.0));
                    changed |= ui.add_sized([width, 30.0], from).changed();
                    ui.label(RichText::new("→").color(p.muted));
                    let to = TextEdit::singleline(&mut rule.to).hint_text("typed").margin(vec2(8.0, 6.0));
                    changed |= ui.add_sized([width, 30.0], to).changed();
                    let (rect, response) = ui.allocate_exact_size(vec2(24.0, 30.0), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(rect, 8.0, p.hover);
                    }
                    ui.painter().text(rect.center(), Align2::CENTER_CENTER, "×", FontId::new(16.0, FontFamily::Proportional), p.muted);
                    if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        remove = Some(i);
                    }
                });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                if ghost(ui, &p, "Add replacement").clicked() {
                    self.config.replacements.push(Replacement::default());
                }
            });
        });
        if let Some(i) = remove {
            self.config.replacements.remove(i);
            changed = true;
        }
        if changed {
            self.save();
        }
    }
}

fn vocabulary_summary(config: &Config) -> String {
    let mut parts = Vec::new();
    match config.presets.len() {
        0 => {}
        1 => parts.push(crate::presets::find(&config.presets[0]).map_or("1 preset".into(), |p| p.name.to_string())),
        n => parts.push(format!("{n} presets")),
    }
    match config.vocabulary.len() {
        0 => {}
        1 => parts.push("1 word".into()),
        n => parts.push(format!("{n} words")),
    }
    if parts.is_empty() { "None".into() } else { parts.join(", ") }
}
