//! One press-to-release cycle: record, transcribe, type. The process lives only this long, so
//! nothing (model included) stays in memory between uses.

use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};

use crate::{
    audio::{Recorder, Tap, WHISPER_RATE},
    config::Config,
    models,
    menu::Page,
    overlay::{Overlay, Phase},
    transcribe::{self, Transcriber},
};

static STOP: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

const TICK: Duration = Duration::from_millis(33);

extern "C" fn on_signal(sig: libc::c_int) {
    if sig == libc::SIGUSR1 {
        STOP.store(true, Ordering::Relaxed);
    } else {
        CANCEL.store(true, Ordering::Relaxed);
    }
}

fn pid_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    dir.join("verse.pid")
}

/// The pid of a running session, if there is one. Checks the process name so a stale
/// pid file whose pid got reused by something else is ignored.
pub fn running_pid() -> Option<i32> {
    let pid: i32 = fs::read_to_string(pid_path()).ok()?.trim().parse().ok()?;
    let theirs = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let ours = fs::read_to_string("/proc/self/comm").ok()?;
    (pid != std::process::id() as i32 && theirs == ours).then_some(pid)
}

/// Sends `sig` to the running session. Waits briefly for one to appear, because on a quick
/// tap the compositor can run `stop` before `start` has written its pid file.
pub fn signal(sig: libc::c_int) -> Result<()> {
    for _ in 0..20 {
        if let Some(pid) = running_pid() {
            unsafe { libc::kill(pid, sig) };
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

pub fn start() -> Result<()> {
    exclusive(record)
}

/// `verse settings [page]`: the pill with its menu open, without recording.
pub fn settings(page: Option<&str>) -> Result<()> {
    let page = match page {
        None => Page::Home,
        Some("model" | "models") => Page::Models,
        Some("language") => Page::Language,
        Some("vocabulary") => Page::Vocabulary,
        Some("replacements") => Page::Replacements,
        Some(other) => anyhow::bail!("no settings page {other:?}"),
    };
    exclusive(|| menu(Overlay::new(Config::load().theme)?, page))
}

/// Runs `session` unless another one is already running, with the pid file and signal
/// handlers that `stop` and `cancel` rely on.
fn exclusive(session: impl FnOnce() -> Result<()>) -> Result<()> {
    if running_pid().is_some() {
        return Ok(());
    }
    // Handlers go in before the pid file exists: SIGUSR1's default action would kill us.
    unsafe {
        for sig in [libc::SIGUSR1, libc::SIGUSR2, libc::SIGINT, libc::SIGTERM] {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }
    fs::write(pid_path(), std::process::id().to_string())?;
    let result = session();
    let _ = fs::remove_file(pid_path());
    if let Err(err) = &result {
        notify(&format!("{err:#}"));
    }
    result
}

/// Shows the menu above the pill until it is closed. The key release that usually ends a
/// recording (`stop`) is ignored here, since the hotkey may still be held when the button is clicked.
fn menu(mut overlay: Overlay, page: Page) -> Result<()> {
    overlay.open_menu(page)?;
    loop {
        overlay.pump(TICK)?;
        if overlay.menu_closed() || overlay.take_button_click() || overlay.closed() || CANCEL.load(Ordering::Relaxed) {
            break;
        }
    }
    overlay.hide();
    Ok(())
}

fn record() -> Result<()> {
    let config = Config::load();
    if !models::is_downloaded(&config.model) {
        // Nothing to transcribe with yet, so go straight to picking a model.
        return menu(Overlay::new(config.theme)?, Page::Models);
    }

    let recorder = Recorder::start()?;
    let transcript = Arc::new(Mutex::new(String::new()));
    let finished = Arc::new(AtomicBool::new(false));
    // Loads the model while you start talking, then keeps transcribing what you've said so far
    // until you let go. It hands the loaded model on for the final pass.
    let worker = thread::spawn({
        let (config, tap) = (config.clone(), recorder.tap());
        let (transcript, finished) = (transcript.clone(), finished.clone());
        move || -> Result<Transcriber> {
            let mut transcriber = transcribe::load(&config)?;
            if config.live_preview {
                preview(&mut transcriber, &config, &tap, &transcript, &finished);
            }
            Ok(transcriber)
        }
    });
    let mut overlay = Overlay::new(config.theme)?;

    while !STOP.load(Ordering::Relaxed) && recorder.seconds() < config.max_seconds as f32 {
        overlay.pump(TICK)?;
        overlay.push_level(recorder.level());
        overlay.set_transcript(&transcript.lock().unwrap())?;
        if overlay.take_button_click() {
            drop(recorder);
            return menu(overlay, Page::Home);
        }
        if CANCEL.load(Ordering::Relaxed) || overlay.closed() {
            return Ok(());
        }
    }

    finished.store(true, Ordering::Relaxed);
    let audio = recorder.finish();
    // A tap this short is a mod + SHIFT combo for something else, not dictation.
    if audio.len() < WHISPER_RATE as usize * 3 / 10 {
        return Ok(());
    }

    overlay.set_phase(Phase::Processing);
    // The preview only ever saw the audio up to its last pass; this one hears all of it.
    let last = thread::spawn(move || {
        let mut transcriber = worker.join().expect("transcription panicked")?;
        transcriber.run(&config, audio)
    });
    while !last.is_finished() {
        overlay.pump(TICK)?;
        // Returning ends the process, worker thread included.
        if CANCEL.load(Ordering::Relaxed) {
            return Ok(());
        }
    }
    let text = last.join().expect("transcription panicked")?;
    overlay.hide();

    if !text.is_empty() {
        type_text(&text)?;
    }
    Ok(())
}

/// Seconds of the most recent audio each preview pass looks at. Whisper works in 30 second
/// windows, and older text is out of the bubble by then anyway.
const PREVIEW_WINDOW: f32 = 28.0;
/// Passes start at most this often, so a fast GPU doesn't spin.
const PREVIEW_INTERVAL: Duration = Duration::from_millis(350);

/// Re-transcribes the recording so far, again and again, until `finished`.
fn preview(transcriber: &mut Transcriber, config: &Config, tap: &Tap, transcript: &Mutex<String>, finished: &AtomicBool) {
    let mut seen = 0;
    while !finished.load(Ordering::Relaxed) {
        let started = Instant::now();
        let len = tap.len();
        let audio = tap.tail(PREVIEW_WINDOW);
        if len == seen || audio.len() < WHISPER_RATE as usize / 2 {
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        seen = len;
        match transcriber.run(config, audio) {
            Ok(text) => *transcript.lock().unwrap() = text,
            // The final pass will run into the same problem and report it.
            Err(_) => return,
        }
        thread::sleep(PREVIEW_INTERVAL.saturating_sub(started.elapsed()));
    }
}

fn type_text(text: &str) -> Result<()> {
    let status = Command::new("wtype")
        .arg("--")
        .arg(text)
        .status()
        .context("running wtype (is it installed?)")?;
    anyhow::ensure!(status.success(), "wtype failed");
    Ok(())
}

fn notify(message: &str) {
    let _ = Command::new("notify-send")
        .args(["-a", "verse", "Verse", message])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
