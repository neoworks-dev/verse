use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};

use anyhow::{Context, Result, bail};
use cpal::{
    FromSample, Sample, SampleFormat, SizedSample, Stream,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

/// The sample rate whisper expects.
pub const WHISPER_RATE: u32 = 16_000;

/// Records the default input device as mono f32 at the device's native rate.
pub struct Recorder {
    /// None when replaying a file instead of the microphone.
    _stream: Option<Stream>,
    samples: Arc<Mutex<Vec<f32>>>,
    level: Arc<AtomicU32>,
    rate: u32,
}

impl Recorder {
    pub fn start() -> Result<Self> {
        // For development: replay a 16 kHz mono 16-bit WAV in real time instead of the mic.
        if let Some(path) = std::env::var_os("VERSE_FAKE_MIC") {
            return Self::replay(std::fs::read(path).context("reading VERSE_FAKE_MIC")?);
        }
        let device = cpal::default_host()
            .default_input_device()
            .context("no input device")?;
        let supported = device.default_input_config().context("no input config")?;
        let rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let format = supported.sample_format();
        let config = supported.into();

        let samples = Arc::new(Mutex::new(Vec::with_capacity(rate as usize * 30)));
        let level = Arc::new(AtomicU32::new(0));

        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, channels, &samples, &level),
            SampleFormat::I16 => build::<i16>(&device, &config, channels, &samples, &level),
            SampleFormat::I32 => build::<i32>(&device, &config, channels, &samples, &level),
            SampleFormat::U16 => build::<u16>(&device, &config, channels, &samples, &level),
            other => bail!("unsupported sample format {other}"),
        }?;
        stream.play()?;

        Ok(Self { _stream: Some(stream), samples, level, rate })
    }

    fn replay(wav: Vec<u8>) -> Result<Self> {
        let audio: Vec<f32> = wav
            .get(44..)
            .context("not a WAV file")?
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        let samples = Arc::new(Mutex::new(Vec::new()));
        let level = Arc::new(AtomicU32::new(0));
        std::thread::spawn({
            let (samples, level) = (samples.clone(), level.clone());
            move || {
                for chunk in audio.chunks(WHISPER_RATE as usize / 50) {
                    let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
                    level.store(rms.to_bits(), Ordering::Relaxed);
                    samples.lock().unwrap().extend_from_slice(chunk);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                level.store(0, Ordering::Relaxed);
            }
        });
        Ok(Self { _stream: None, samples, level, rate: WHISPER_RATE })
    }

    /// RMS of the most recent callback's audio, roughly 0..1.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    pub fn seconds(&self) -> f32 {
        self.samples.lock().unwrap().len() as f32 / self.rate as f32
    }

    /// A handle another thread can read the recording so far through.
    pub fn tap(&self) -> Tap {
        Tap { samples: self.samples.clone(), rate: self.rate }
    }

    /// Stops recording and returns the audio resampled to 16 kHz.
    pub fn finish(self) -> Vec<f32> {
        let rate = self.rate;
        drop(self._stream);
        let samples = std::mem::take(&mut *self.samples.lock().unwrap());
        resample(&samples, rate, WHISPER_RATE)
    }
}

#[derive(Clone)]
pub struct Tap {
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
}

impl Tap {
    /// How much has been recorded, in native samples. Cheap, for noticing new audio.
    pub fn len(&self) -> usize {
        self.samples.lock().unwrap().len()
    }

    /// The last `seconds` of the recording, resampled to 16 kHz.
    pub fn tail(&self, seconds: f32) -> Vec<f32> {
        let tail = {
            let samples = self.samples.lock().unwrap();
            let keep = ((seconds * self.rate as f32) as usize).min(samples.len());
            samples[samples.len() - keep..].to_vec()
        };
        resample(&tail, self.rate, WHISPER_RATE)
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    samples: &Arc<Mutex<Vec<f32>>>,
    level: &Arc<AtomicU32>,
) -> Result<Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let samples = samples.clone();
    let level = level.clone();
    let stream = device.build_input_stream(
        config.clone(),
        move |data: &[T], _: &_| {
            let mut sum = 0.0;
            let mut out = samples.lock().unwrap();
            for frame in data.chunks_exact(channels) {
                let mono = frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32;
                sum += mono * mono;
                out.push(mono);
            }
            let frames = (data.len() / channels).max(1);
            level.store((sum / frames as f32).sqrt().to_bits(), Ordering::Relaxed);
        },
        |err| eprintln!("audio stream error: {err}"),
        None,
    )?;
    Ok(stream)
}

/// Linear resampling. Plenty for speech going into whisper, which works at 16 kHz anyway.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let len = (input.len() as f64 / ratio) as usize;
    (0..len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx];
            let b = *input.get(idx + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}
