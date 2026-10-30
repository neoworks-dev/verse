use anyhow::{Context, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

use crate::{audio::WHISPER_RATE, config::Config, models, vocab};

/// A loaded model with its working buffers, reused for every pass: the live previews while
/// recording and the final transcription after it.
pub struct Transcriber {
    state: WhisperState,
}

pub fn load(config: &Config) -> Result<Transcriber> {
    whisper_rs::install_logging_hooks();
    let path = models::path(&config.model);
    let mut params = WhisperContextParameters::default();
    params.use_gpu(config.use_gpu);
    let ctx = WhisperContext::new_with_params(path.to_str().context("model path is not UTF-8")?, params)
        .with_context(|| format!("loading {}", path.display()))?;
    // The state holds on to the model, so the context itself can go.
    Ok(Transcriber { state: ctx.create_state()? })
}

impl Transcriber {
    pub fn run(&mut self, config: &Config, mut audio: Vec<f32>) -> Result<String> {
        // whisper.cpp wants at least a second of audio; pad short clips with silence.
        let min = WHISPER_RATE as usize + WHISPER_RATE as usize / 10;
        if audio.len() < min {
            audio.resize(min, 0.0);
        }

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let language = (config.language != "auto").then_some(config.language.as_str());
        params.set_language(language.or(Some("auto")));
        params.set_n_threads(std::thread::available_parallelism().map_or(4, |n| n.get().min(8)) as i32);
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        let terms = config.terms();
        let prompt = vocab::prompt(&terms);
        if let Some(prompt) = &prompt {
            params.set_initial_prompt(prompt);
        }

        self.state.full(params, &audio)?;

        let mut text = String::new();
        for segment in self.state.as_iter() {
            let part = segment.to_str_lossy()?;
            let part = part.trim();
            if part.is_empty() || is_annotation(part) {
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(part);
        }
        Ok(vocab::correct(&text, &terms, &config.replacements))
    }
}

/// Whisper marks silence and noise with things like "[BLANK_AUDIO]" or "(wind blowing)".
fn is_annotation(s: &str) -> bool {
    (s.starts_with('[') && s.ends_with(']')) || (s.starts_with('(') && s.ends_with(')'))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// Downloads tiny.en and transcribes a 16 kHz mono 16-bit WAV given in `TRANSCRIBE_WAV`.
    #[test]
    #[ignore = "downloads a model; run with --ignored"]
    fn transcribes_wav() {
        let config = Config { model: "tiny.en".into(), language: "en".into(), ..Config::default() };
        if !models::is_downloaded(&config.model) {
            models::download(&config.model, Arc::new(models::Progress::default())).unwrap();
        }
        let wav = std::fs::read(std::env::var("TRANSCRIBE_WAV").unwrap()).unwrap();
        let audio = wav[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        let text = load(&config).unwrap().run(&config, audio).unwrap();
        println!("{text}");
        assert!(text.to_lowercase().contains("ask not"));
    }
}
