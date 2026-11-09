//! Speech recognition on the device with whisper.cpp.
//!
//! The model file (ggml format, as voxtype uses it) lies on the device; audio never leaves it.
//! Audio comes in as 16 kHz mono 16-bit PCM, the format Android's recorder delivers.

use std::sync::Mutex;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("model could not be loaded: {0}")]
    Load(String),
    #[error("recognition failed: {0}")]
    Recognize(String),
}

pub type Result<T> = std::result::Result<T, VoiceError>;

/// A loaded model. Loading takes a while (hundreds of MB), so the app keeps one around.
pub struct Whisper {
    ctx: WhisperContext,
    /// whisper.cpp states are not meant to run twice at once.
    busy: Mutex<()>,
}

impl Whisper {
    pub fn load(path: &str) -> Result<Self> {
        let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|e| VoiceError::Load(e.to_string()))?;
        Ok(Self {
            ctx,
            busy: Mutex::new(()),
        })
    }

    /// Text of `pcm` (16 kHz mono, little-endian i16). `language` like `de` or `de-CH`
    /// (empty = detect); `prompt` lists words to expect, like voxtype's `initial_prompt`.
    pub fn transcribe(&self, pcm: &[u8], language: &str, prompt: &str) -> Result<String> {
        let _guard = self.busy.lock().unwrap_or_else(|e| e.into_inner());
        let samples: Vec<f32> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b) as f32 / 32768.0)
            .collect();
        if samples.len() < 16_000 / 4 {
            return Ok(String::new()); // under a quarter second: nothing said
        }
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| VoiceError::Recognize(e.to_string()))?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let lang = language_code(language);
        params.set_language(lang.as_deref().or(Some("auto")));
        if !prompt.trim().is_empty() {
            params.set_initial_prompt(prompt);
        }
        params.set_n_threads(threads());
        let ctx = audio_ctx(samples.len());
        if ctx > 0 {
            params.set_audio_ctx(ctx);
        }
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_suppress_blank(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        state
            .full(params, &samples)
            .map_err(|e| VoiceError::Recognize(e.to_string()))?;
        let mut text = String::new();
        for segment in state.as_iter() {
            if let Ok(s) = segment.to_str_lossy() {
                text.push_str(&s);
            }
        }
        Ok(clean(&text))
    }
}

/// `de-CH` → `de`; empty → None (whisper detects the language).
fn language_code(tag: &str) -> Option<String> {
    let code = tag
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    (!code.is_empty()).then_some(code)
}

/// Encoder window for a take of `samples`: whisper always encodes 30 s (1500 frames); a short
/// take needs only its own length plus a margin, which makes it several times faster.
/// `MC_VOICE_AUDIO_CTX=0` switches this off (for comparisons).
fn audio_ctx(samples: usize) -> i32 {
    if std::env::var("MC_VOICE_AUDIO_CTX").is_ok_and(|v| v == "0") {
        return 0;
    }
    let frames = samples / 320 + 64; // 50 frames per second, plus about a second
    if frames >= 1500 {
        0
    } else {
        frames.max(256) as i32
    }
}

fn threads() -> i32 {
    if let Some(n) = std::env::var("MC_VOICE_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return n;
    }
    std::thread::available_parallelism()
        .map(|n| n.get().clamp(2, 6) as i32)
        .unwrap_or(4)
}

/// Whisper marks silence and noise as `[BLANK_AUDIO]`, `(Musik)` and the like.
/// Only such markers go: a closed bracket standing on its own, `[…]` with
/// anything in it, `(…)` with a single word. An open bracket, `foo(bar)` and
/// `(ausser .env)` are part of what was said.
fn clean(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match closing_bracket(&chars, i).filter(|&end| is_marker(&chars, i, end)) {
            Some(end) => i = end + 1,
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    // A marker right before a full stop leaves the stop as a word of its own: join it back.
    let mut joined = String::new();
    for w in words {
        let stop = w
            .chars()
            .all(|c| matches!(c, '.' | ',' | '!' | '?' | ';' | ':'));
        if !joined.is_empty() && !stop {
            joined.push(' ');
        }
        joined.push_str(w);
    }
    joined
}

/// Whether the brackets `start..=end` are a noise marker (see [`clean`]).
fn is_marker(chars: &[char], start: usize, end: usize) -> bool {
    let alone_before = start == 0 || chars[start - 1].is_whitespace();
    let alone_after = chars
        .get(end + 1)
        .is_none_or(|c| c.is_whitespace() || matches!(c, '.' | ',' | '!' | '?' | ';' | ':'));
    if !alone_before || !alone_after {
        return false;
    }
    let inner: String = chars[start + 1..end].iter().collect();
    let inner = inner.trim();
    chars[start] == '[' || (!inner.is_empty() && inner.chars().all(char::is_alphabetic))
}

/// Index of the bracket that closes the one opened at `start`, if any.
fn closing_bracket(chars: &[char], start: usize) -> Option<usize> {
    if !matches!(chars[start], '[' | '(') {
        return None;
    }
    let mut depth = 0;
    for (i, ch) in chars.iter().enumerate().skip(start) {
        match ch {
            '[' | '(' => depth += 1,
            ']' | ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes() {
        assert_eq!(language_code("de-CH").as_deref(), Some("de"));
        assert_eq!(language_code(""), None);
    }

    #[test]
    fn cleans_markers() {
        assert_eq!(clean(" [BLANK_AUDIO] Hallo  (Musik) Welt "), "Hallo Welt");
        assert_eq!(clean("a [x (y) z] b"), "a b");
    }

    #[test]
    fn keeps_brackets_that_were_said() {
        assert_eq!(
            clean("Lösche die Dateien (ausser .env)"),
            "Lösche die Dateien (ausser .env)"
        );
        assert_eq!(clean("ruf foo(bar) auf"), "ruf foo(bar) auf");
        assert_eq!(clean("das (zweite Mal) gilt"), "das (zweite Mal) gilt");
        assert_eq!(clean("Ende (lacht)."), "Ende.");
    }

    #[test]
    fn keeps_unclosed_brackets() {
        assert_eq!(clean("Hallo (ich"), "Hallo (ich");
        assert_eq!(clean("Liste [eins (Musik) zwei"), "Liste [eins zwei");
        assert_eq!(clean("a) b ]"), "a) b ]");
    }
}
