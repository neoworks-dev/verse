use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, bail};

use crate::config::data_dir;

pub struct Model {
    pub name: &'static str,
    pub size_mb: u32,
    pub description: &'static str,
}

pub const DEFAULT: &str = "large-v3-turbo-q5_0";

/// whisper.cpp ggml models, all hosted at huggingface.co/ggerganov/whisper.cpp.
pub const CATALOG: &[Model] = &[
    Model { name: "tiny.en", size_mb: 75, description: "English, fastest, least accurate" },
    Model { name: "base.en", size_mb: 142, description: "English, fast on any CPU" },
    Model { name: "base", size_mb: 142, description: "Multilingual, fast on any CPU" },
    Model { name: "small.en", size_mb: 466, description: "English, good on a CPU" },
    Model { name: "small", size_mb: 466, description: "Multilingual, good on a CPU" },
    Model { name: "large-v3-turbo-q5_0", size_mb: 547, description: "Multilingual, best with a GPU" },
    Model { name: "large-v3-turbo", size_mb: 1624, description: "Multilingual, needs a GPU" },
];

pub fn models_dir() -> PathBuf {
    data_dir().join("models")
}

pub fn path(name: &str) -> PathBuf {
    models_dir().join(format!("ggml-{name}.bin"))
}

pub fn is_downloaded(name: &str) -> bool {
    path(name).is_file()
}

fn url(name: &str) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin")
}

#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

/// Downloads a model to a `.part` file and renames it into place once complete,
/// so a half-finished download never looks like a usable model.
pub fn download(name: &str, progress: Arc<Progress>) -> Result<()> {
    fs::create_dir_all(models_dir())?;
    let dest = path(name);
    let part = dest.with_extension("bin.part");

    let response = ureq::get(&url(name)).call().context("requesting model")?;
    let mut body = response.into_body();
    progress.total.store(body.content_length().unwrap_or(0), Ordering::Relaxed);

    let mut reader = body.as_reader();
    let mut file = fs::File::create(&part)?;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if progress.cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = fs::remove_file(&part);
            bail!("cancelled");
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        progress.done.fetch_add(n as u64, Ordering::Relaxed);
    }
    file.sync_all()?;
    fs::rename(&part, &dest)?;
    Ok(())
}

pub fn delete(name: &str) -> Result<()> {
    fs::remove_file(path(name)).context("deleting model")
}
