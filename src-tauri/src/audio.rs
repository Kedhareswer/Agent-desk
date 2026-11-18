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

/// Where a continuous listener takes its sound from.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Source {
    Microphone,
    /// Whatever the computer is playing (the other side of a call).
    /// Windows only: it records the default speakers "in loopback".
    System,
}

/// Continuous capture for live mode. Audio arrives in small pieces, already
/// converted to 16 kHz mono. Dropping the listener stops it.
pub struct Listener {
    stop: mpsc::Sender<bool>,
}

impl Listener {
    pub fn start(source: Source, on_audio: impl Fn(&[f32]) + Send + 'static) -> Result<Self, String> {
        let (stop, stopped) = mpsc::channel::<bool>();
        let (ready, is_ready) = mpsc::channel::<Result<(), String>>();
        std::thread::spawn(move || {
            let opened = (|| -> Result<cpal::Stream, String> {
                let host = cpal::default_host();
                let (device, config) = match source {
                    Source::Microphone => {
                        let d = host.default_input_device().ok_or("No microphone was found.")?;
                        let c = d.default_input_config().map_err(|e| format!("The microphone could not be opened: {e}"))?;
                        (d, c)
                    }
                    Source::System => {
                        let d = host.default_output_device().ok_or("No speakers were found to listen to.")?;
                        let c = d.default_output_config().map_err(|e| format!("The computer's audio could not be opened: {e}"))?;
                        (d, c)
                    }
                };
                let channels = config.channels() as usize;
                let rate = config.sample_rate();
                let push = move |chunk: &[f32]| on_audio(&to_16k_mono(chunk, channels, rate));
                let on_error = |e| eprintln!("agent-desk: audio error: {e}");
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
                    other => return Err(format!("Unsupported audio format ({other:?}).")),
                }
                .map_err(|e| format!("Audio capture could not start: {e}"))?;
                stream.play().map_err(|e| format!("Audio capture could not start: {e}"))?;
                Ok(stream)
            })();
            match opened {
                Ok(stream) => {
                    let _ = ready.send(Ok(()));
                    let _ = stopped.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready.send(Err(e));
                }
            }
        });
        is_ready.recv_timeout(std::time::Duration::from_secs(5)).map_err(|_| "Audio capture did not start in time.".to_string())??;
        Ok(Self { stop })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

const FRAME: usize = 480; // 30 ms at 16 kHz
const SPEECH_RMS: f32 = 0.012;
const START_FRAMES: usize = 3; // 90 ms of sound starts an utterance
const END_FRAMES: usize = 24; // 720 ms of quiet ends it
const PRE_ROLL_FRAMES: usize = 10; // keep 300 ms before speech started
const MIN_SPEECH_FRAMES: usize = 12; // ignore sounds shorter than 360 ms
const MAX_FRAMES: usize = 667; // cut very long speech at about 20 s

/// Splits a continuous stream into utterances: stretches of speech
/// separated by pauses. Each finished utterance is handed back whole.
#[derive(Default)]
pub struct Segmenter {
    pending: Vec<f32>,   // samples not yet forming a whole frame
    pre_roll: Vec<f32>,  // the most recent quiet frames
    utterance: Vec<f32>, // the speech being collected
    loud_run: usize,
    quiet_run: usize,
    loud_frames: usize,
    speaking: bool,
}

impl Segmenter {
    /// Feed 16 kHz mono audio. Returns any utterances that just ended.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        let mut finished = Vec::new();
        self.pending.extend_from_slice(samples);
        let whole = self.pending.len() / FRAME * FRAME;
        let frames: Vec<f32> = self.pending.drain(..whole).collect();
        for frame in frames.chunks_exact(FRAME) {
            let loud = (frame.iter().map(|s| s * s).sum::<f32>() / FRAME as f32).sqrt() > SPEECH_RMS;
            if !self.speaking {
                self.pre_roll.extend_from_slice(frame);
                let keep = PRE_ROLL_FRAMES * FRAME;
                if self.pre_roll.len() > keep {
                    self.pre_roll.drain(..self.pre_roll.len() - keep);
                }
                self.loud_run = if loud { self.loud_run + 1 } else { 0 };
                if self.loud_run >= START_FRAMES {
                    self.speaking = true;
                    self.utterance = std::mem::take(&mut self.pre_roll);
                    self.loud_frames = self.loud_run;
                    self.quiet_run = 0;
                }
                continue;
            }
            self.utterance.extend_from_slice(frame);
            if loud {
                self.loud_frames += 1;
                self.quiet_run = 0;
            } else {
                self.quiet_run += 1;
            }
            if self.quiet_run >= END_FRAMES || self.utterance.len() >= MAX_FRAMES * FRAME {
                let done = std::mem::take(&mut self.utterance);
                if self.loud_frames >= MIN_SPEECH_FRAMES {
                    finished.push(done);
                }
                self.speaking = false;
                self.loud_run = 0;
                self.loud_frames = 0;
            }
        }
        finished
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

    fn tone(ms: usize) -> Vec<f32> {
        (0..ms * 16).map(|i| (i as f32 * 0.1).sin() * 0.2).collect()
    }
    fn quiet(ms: usize) -> Vec<f32> {
        vec![0.0005; ms * 16]
    }

    #[test]
    fn speech_separated_by_pauses_becomes_separate_utterances() {
        let mut seg = Segmenter::default();
        let mut audio = quiet(500);
        audio.extend(tone(1200));
        audio.extend(quiet(1000));
        audio.extend(tone(900));
        audio.extend(quiet(1000));
        // Feed in uneven pieces, as a sound card would.
        let found: Vec<Vec<f32>> = audio.chunks(733).flat_map(|c| seg.push(c)).collect();
        assert_eq!(found.len(), 2);
        // Each has its speech plus a little lead-in and the closing pause.
        let secs: Vec<f32> = found.iter().map(|u| u.len() as f32 / 16_000.0).collect();
        assert!((1.9..2.4).contains(&secs[0]), "{secs:?}");
        assert!((1.6..2.1).contains(&secs[1]), "{secs:?}");
    }

    #[test]
    fn short_pauses_do_not_split_a_sentence() {
        let mut seg = Segmenter::default();
        let mut audio = tone(800);
        audio.extend(quiet(300)); // a breath
        audio.extend(tone(800));
        audio.extend(quiet(900));
        assert_eq!(seg.push(&audio).len(), 1);
    }

    #[test]
    fn clicks_and_silence_produce_nothing() {
        let mut seg = Segmenter::default();
        let mut audio = quiet(1000);
        audio.extend(tone(120)); // a cough or key press
        audio.extend(quiet(2000));
        assert!(seg.push(&audio).is_empty());
    }

    #[test]
    fn endless_speech_is_cut_into_pieces() {
        let mut seg = Segmenter::default();
        let found = seg.push(&tone(45_000));
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|u| u.len() <= MAX_FRAMES * FRAME));
    }
}
