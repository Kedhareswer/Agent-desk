//! Microphone capture for push-to-talk.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    sync::{mpsc, Arc, Mutex},
    thread::JoinHandle,
};

pub const TARGET_RATE: u32 = 16_000;

/// A recording in progress. The audio stream lives on its own thread
/// because the operating system's stream handle cannot move between threads.
pub struct Recording {
    stop: mpsc::Sender<bool>,
    worker: JoinHandle<Result<Vec<f32>, String>>,
}

impl Recording {
    /// Start recording from the default microphone. `level` is called a few
    /// dozen times a second with the current loudness (0.0 to 1.0).
    pub fn start(level: impl Fn(f32) + Send + 'static) -> Self {
        let (stop, stopped) = mpsc::channel::<bool>();
        let worker = std::thread::spawn(move || {
            let device = cpal::default_host().default_input_device().ok_or("No microphone was found.")?;
            let config = device.default_input_config().map_err(|e| format!("The microphone could not be opened: {e}"))?;
            let channels = config.channels() as usize;
            let rate = config.sample_rate();
            let buffer = Arc::new(Mutex::new(Vec::<f32>::new()));

            let sink = buffer.clone();
            let push = move |chunk: &[f32]| {
                level(loudness(chunk));
                if let Ok(mut b) = sink.lock() {
                    b.extend_from_slice(chunk);
                }
            };
            let on_error = |e| eprintln!("agent-desk: microphone error: {e}");
            let format = config.sample_format();
            let stream_config: cpal::StreamConfig = config.into();
            let stream = match format {
                cpal::SampleFormat::F32 => device.build_input_stream(stream_config, move |d: &[f32], _: &_| push(d), on_error, None),
                cpal::SampleFormat::I16 => device.build_input_stream(
                    stream_config,
                    move |d: &[i16], _: &_| push(&d.iter().map(|s| *s as f32 / 32768.0).collect::<Vec<_>>()),
                    on_error,
                    None,
                ),
                cpal::SampleFormat::U16 => device.build_input_stream(
                    stream_config,
                    move |d: &[u16], _: &_| push(&d.iter().map(|s| (*s as f32 - 32768.0) / 32768.0).collect::<Vec<_>>()),
                    on_error,
                    None,
                ),
                other => return Err(format!("The microphone uses an unsupported format ({other:?}).")),
            }
            .map_err(|e| format!("The microphone could not be opened: {e}. Check Windows microphone privacy settings."))?;
            stream.play().map_err(|e| format!("The microphone could not start: {e}"))?;

            let keep = stopped.recv().unwrap_or(false);
            drop(stream);
            if !keep {
                return Ok(Vec::new());
            }
            let raw = std::mem::take(&mut *buffer.lock().map_err(|_| "audio buffer lock poisoned")?);
            Ok(to_16k_mono(&raw, channels, rate))
        });
        Self { stop, worker }
    }

    /// Stop and return the audio as 16 kHz mono.
    pub fn finish(self) -> Result<Vec<f32>, String> {
        let _ = self.stop.send(true);
        self.worker.join().map_err(|_| "The recording thread crashed.".to_string())?
    }

    /// Stop and throw the audio away.
    pub fn cancel(self) {
        let _ = self.stop.send(false);
    }
}

/// Root-mean-square loudness of a chunk, scaled so normal speech lands
/// around 0.3 to 0.8.
pub fn loudness(chunk: &[f32]) -> f32 {
    if chunk.is_empty() {
        return 0.0;
    }
    let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
    (rms * 6.0).min(1.0)
}

/// Mix to one channel and resample to 16 kHz (linear interpolation, which
/// is plenty for speech recognition).
pub fn to_16k_mono(samples: &[f32], channels: usize, rate: u32) -> Vec<f32> {
    let channels = channels.max(1);
    let mono: Vec<f32> = samples.chunks_exact(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect();
    if rate == TARGET_RATE || mono.is_empty() {
        return mono;
    }
    let ratio = rate as f64 / TARGET_RATE as f64;
    let out_len = (mono.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let a = pos as usize;
            let b = (a + 1).min(mono.len() - 1);
            let t = (pos - a as f64) as f32;
            mono[a] * (1.0 - t) + mono[b] * t
        })
        .collect()
}

/// Whether a 16 kHz recording contains anything louder than room noise.
/// Whisper invents words for pure silence, so silent clips are not sent to it.
pub fn has_speech(samples: &[f32]) -> bool {
    const WINDOW: usize = 480; // 30 ms
    let loud = samples.chunks(WINDOW).filter(|w| (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt() > 0.012).count();
    loud >= 5 // at least 150 ms of sound
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_48k_becomes_mono_16k() {
        // One second of stereo at 48 kHz: left = 1.0, right = 0.0.
        let stereo: Vec<f32> = (0..48_000).flat_map(|_| [1.0, 0.0]).collect();
        let out = to_16k_mono(&stereo, 2, 48_000);
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn odd_rates_keep_duration() {
        let out = to_16k_mono(&vec![0.1; 44_100 * 2], 1, 44_100);
        assert!((out.len() as i64 - 32_000).abs() <= 1);
    }

    #[test]
    fn silence_and_clicks_are_not_speech() {
        assert!(!has_speech(&vec![0.0; 32_000]));
        let mut click = vec![0.001; 32_000];
        click[100..300].fill(0.5); // a 12 ms key click
        assert!(!has_speech(&click));
        let tone: Vec<f32> = (0..16_000).map(|i| (i as f32 * 0.1).sin() * 0.2).collect();
        assert!(has_speech(&tone));
    }
}
