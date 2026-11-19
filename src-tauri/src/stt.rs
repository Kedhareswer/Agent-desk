//! Speech-to-text, fully on this computer, with whisper.cpp.
//!
//! Neither Claude nor Codex accepts audio, so speech is turned into text
//! locally first. The model file is downloaded once (on first use) and is
//! only held in memory while voice is being used.

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::Instant,
};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

/// (settings name, file name, size in bytes) of the models offered.
pub const MODELS: &[(&str, &str, u64)] = &[
    ("tiny.en", "ggml-tiny.en.bin", 77_704_715),
    ("base.en", "ggml-base.en.bin", 147_964_211),
    ("small.en", "ggml-small.en.bin", 487_614_201),
    ("base", "ggml-base.bin", 147_951_465),
    ("small", "ggml-small.bin", 487_601_967),
];
const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

fn lookup(name: &str) -> (&'static str, u64) {
    MODELS
        .iter()
        .find(|m| m.0 == name)
        .map(|m| (m.1, m.2))
        .unwrap_or((MODELS[1].1, MODELS[1].2))
}

pub fn model_path(dir: &Path, name: &str) -> PathBuf {
    dir.join("models").join(lookup(name).0)
}

/// Whether the model is fully downloaded.
pub fn model_ready(dir: &Path, name: &str) -> bool {
    let size = lookup(name).1;
    std::fs::metadata(model_path(dir, name)).is_ok_and(|m| m.len() == size)
}

/// Download the model if it is missing. `progress` receives 0..=100.
/// Uses the `curl` that ships with Windows 10 and later, so the app needs
/// no networking code of its own.
pub fn ensure_model(dir: &Path, name: &str, progress: &dyn Fn(u8)) -> Result<PathBuf, String> {
    let (file, size) = lookup(name);
    let path = model_path(dir, name);
    if model_ready(dir, name) {
        return Ok(path);
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let partial = path.with_extension("part");
    let mut cmd = Command::new("curl");
    cmd.args([
        "--location",
        "--fail",
        "--silent",
        "--show-error",
        "--continue-at",
        "-",
        "--output",
    ])
    .arg(&partial)
    .arg(format!("{BASE_URL}/{file}"));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start the download (curl): {e}"))?;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        let done = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
        progress((done.saturating_mul(100) / size.max(1)).min(99) as u8);
        std::thread::sleep(std::time::Duration::from_millis(400));
    };
    let got = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
    if !status.success() || got != size {
        return Err("The speech model could not be downloaded. Check your internet connection and try again.".into());
    }
    std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    progress(100);
    Ok(path)
}

struct Loaded {
    name: String,
    ctx: WhisperContext,
    /// Whisper's working memory, made once and reused: creating it costs
    /// tens of milliseconds on every question otherwise.
    state: Option<WhisperState>,
}

/// How much of Whisper's 30-second window to run for a clip of `samples`.
/// Whisper always works on 30 s (1500 frames of 20 ms) even for a 3-word
/// question; running only what the clip needs, plus a margin, makes the
/// slowest step several times faster on short questions.
pub fn audio_ctx(samples: usize) -> i32 {
    let frames = samples.div_ceil(320) + 128; // 320 samples = 20 ms; margin 2.5 s
    (frames.max(512).div_ceil(64) * 64).min(1500) as i32
}

#[derive(Default)]
pub struct Transcriber {
    loaded: Mutex<Option<Loaded>>,
    last_used: Mutex<Option<Instant>>,
}

impl Transcriber {
    /// Load the model into memory (a no-op when it already is).
    pub fn load(&self, dir: &Path, name: &str) -> Result<(), String> {
        let mut slot = self
            .loaded
            .lock()
            .map_err(|_| "speech engine lock poisoned")?;
        *self.last_used.lock().unwrap() = Some(Instant::now());
        if slot.as_ref().is_some_and(|l| l.name == name) {
            return Ok(());
        }
        let path = model_path(dir, name);
        let ctx = WhisperContext::new_with_params(&path, WhisperContextParameters::default())
            .map_err(|e| format!("Could not load the speech model: {e}"))?;
        *slot = Some(Loaded {
            name: name.to_string(),
            ctx,
            state: None,
        });
        Ok(())
    }

    /// Turn 16 kHz mono audio into text. `load` must have succeeded first.
    pub fn transcribe(&self, samples: &[f32]) -> Result<String, String> {
        let mut slot = self
            .loaded
            .lock()
            .map_err(|_| "speech engine lock poisoned")?;
        let loaded = slot.as_mut().ok_or("The speech model is not loaded.")?;
        *self.last_used.lock().unwrap() = Some(Instant::now());
        // Holding the keys a moment before and after speaking is common;
        // that silence is only extra work.
        let samples = crate::audio::trim_silence(samples);

        if loaded.state.is_none() {
            loaded.state = Some(loaded.ctx.create_state().map_err(|e| e.to_string())?);
        }
        let english = loaded.name.ends_with(".en");
        let state = loaded.state.as_mut().expect("just made");
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(1, 8);
        params.set_n_threads(threads as i32);
        params.set_language(if english { Some("en") } else { None });
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_no_context(true);
        params.set_single_segment(samples.len() < 16_000 * 25);

        // Whisper needs at least a second of audio; pad short clips with silence.
        let mut audio = samples.to_vec();
        if audio.len() < 16_000 + 1_600 {
            audio.resize(16_000 + 1_600, 0.0);
        }
        params.set_audio_ctx(audio_ctx(audio.len()));
        state
            .full(params, &audio)
            .map_err(|e| format!("Speech recognition failed: {e}"))?;

        let mut text = String::new();
        for segment in state.as_iter() {
            text.push_str(&segment.to_string());
        }
        Ok(clean(&text))
    }

    /// Free the model after it has not been used for a while.
    pub fn reap_idle(&self, minutes: u32) {
        let idle = self
            .last_used
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed().as_secs() >= u64::from(minutes) * 60);
        if idle {
            if let Ok(mut slot) = self.loaded.try_lock() {
                *slot = None;
            }
        }
    }
}

/// Whisper marks silence and noise with bracketed tags such as
/// "[BLANK_AUDIO]" or "(wind blowing)". Those are not something the user said.
pub fn clean(raw: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for c in raw.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_clips_use_a_shorter_window() {
        // A 3-second question runs about a third of the full window.
        assert_eq!(audio_ctx(3 * 16_000), 512);
        assert_eq!(audio_ctx(10 * 16_000), 640);
        // Never more than Whisper's own 30 s, and a multiple of 64.
        assert_eq!(audio_ctx(40 * 16_000), 1500);
        for secs in 1..30 {
            let n = audio_ctx(secs * 16_000);
            assert!(n == 1500 || n % 64 == 0);
            assert!(
                n as usize >= secs * 50 + 128 || n == 1500,
                "covers the clip"
            );
        }
    }

    #[test]
    fn noise_tags_are_removed() {
        assert_eq!(clean(" [BLANK_AUDIO] "), "");
        assert_eq!(
            clean(" Draft a reply  (keyboard clicking) to Ben."),
            "Draft a reply to Ben."
        );
    }

    #[test]
    fn unknown_model_falls_back_to_base_english() {
        assert!(model_path(Path::new("/x"), "nope").ends_with("models/ggml-base.en.bin"));
    }

    /// Real transcription. Needs a model, a 16 kHz mono WAV and a phrase
    /// that the recording contains:
    ///   AGENT_DESK_MODELS=<dir containing models/> AGENT_DESK_WAV=<file> \
    ///   AGENT_DESK_EXPECT="draft a reply" cargo test -- --ignored live_whisper --nocapture
    #[test]
    #[ignore]
    fn live_whisper_transcribes_speech() {
        let dir = PathBuf::from(std::env::var("AGENT_DESK_MODELS").expect("AGENT_DESK_MODELS"));
        let wav = std::env::var("AGENT_DESK_WAV").expect("AGENT_DESK_WAV");
        let mut reader = hound::WavReader::open(wav).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();

        let t = Transcriber::default();
        let started = Instant::now();
        t.load(&dir, "base.en").unwrap();
        eprintln!("model loaded in {:?}", started.elapsed());
        let started = Instant::now();
        let text = t.transcribe(&samples).unwrap();
        eprintln!(
            "{:.1}s of audio transcribed in {:?}: {text}",
            samples.len() as f32 / 16000.0,
            started.elapsed()
        );
        let expect = std::env::var("AGENT_DESK_EXPECT")
            .expect("AGENT_DESK_EXPECT")
            .to_lowercase();
        assert!(text.to_lowercase().contains(&expect), "{text}");

        // Silence must come back empty, not as an invented sentence.
        let silence = t.transcribe(&vec![0.0; 16_000 * 2]).unwrap();
        eprintln!("silence -> {silence:?}");
        assert!(silence.len() < 4, "{silence}");
    }
}
