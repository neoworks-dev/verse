use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{models, presets, vocab::Replacement};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Name of a model from `models::CATALOG`.
    pub model: String,
    /// Whisper language code, or "auto" to detect it.
    pub language: String,
    pub theme: Theme,
    /// Recording stops on its own after this long, in case the key release never arrives.
    pub max_seconds: u32,
    pub use_gpu: bool,
    /// Show what whisper hears while you talk, above the pill.
    pub live_preview: bool,
    /// Words whisper should know: names, jargon, code identifiers.
    pub vocabulary: Vec<String>,
    /// Ids of the `presets::PRESETS` whose terms are added to the vocabulary.
    pub presets: Vec<String>,
    /// Applied to the transcript last, as whole words and ignoring case.
    pub replacements: Vec<Replacement>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: models::DEFAULT.to_string(),
            language: "auto".to_string(),
            theme: Theme::Dark,
            max_seconds: 120,
            use_gpu: true,
            live_preview: true,
            vocabulary: Vec::new(),
            presets: Vec::new(),
            replacements: Vec::new(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("verse")
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("verse")
}

fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

impl Config {
    /// The user's own words first, so they win the space in whisper's short prompt, then the
    /// selected presets, without duplicates.
    pub fn terms(&self) -> Vec<String> {
        let presets = self.presets.iter().filter_map(|id| presets::find(id));
        let mut terms: Vec<String> = Vec::new();
        for term in self.vocabulary.iter().map(String::as_str).chain(presets.flat_map(|p| p.terms.iter().copied())) {
            if !terms.iter().any(|t| t.eq_ignore_ascii_case(term)) {
                terms.push(term.to_string());
            }
        }
        terms
    }

    /// Reads the config, falling back to defaults when it is missing or unreadable.
    pub fn load() -> Self {
        Self::load_checked().0
    }

    /// Like `load`, but also says why an existing config couldn't be used, so the menu can
    /// show it instead of saving defaults over the user's file.
    pub fn load_checked() -> (Self, Option<String>) {
        let Ok(text) = fs::read_to_string(config_path()) else {
            return (Self::default(), None);
        };
        match toml::from_str(&text) {
            Ok(config) => (config, None),
            Err(err) => {
                let path = config_path().display().to_string();
                let path = dirs::home_dir()
                    .and_then(|home| path.strip_prefix(&*home.to_string_lossy()).map(|rest| format!("~{rest}")))
                    .unwrap_or(path);
                (Self::default(), Some(format!("{path} is invalid: {}", err.message())))
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(config_dir())?;
        let text = toml::to_string_pretty(self)?;
        fs::write(config_path(), text).context("writing config")
    }
}
