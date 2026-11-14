// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The monitor pipeline: frame in, scrubbed text out (or a reason it was skipped).
//!
//! Order of checks, cheapest and most protective first: monitor on? paused? time since the last observation? sensitive
//! window (before any pixel is read)? near-duplicate of the last kept frame? Then the vision engine describes the frame,
//! the description is scrubbed ([`crate::privacy::redact`]) and the frame is dropped. Every decision is reported to the
//! [`Audit`] sink as a reason only, never as content.

use crate::privacy::{redact, sensitive_window};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const MAX_DIMENSION: u32 = 16_384;
pub const MAX_TEXT_CHARS: usize = 4_000;
/// the longest side to capture when the text on the screen is to be read
pub const MIN_EDGE_FOR_OCR: u32 = 2560;

/// One captured screen, handed over by the capture plug-in as raw RGBA pixels with the focused window's identity.
pub struct Frame {
    pub ts_ms: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub app: String,
    pub title: String,
}

/// Turns a frame into a text description (a vision-capable engine plug-in).
pub trait Describer: Send + Sync {
    fn describe(&self, frame: &Frame) -> Result<String, String>;
}

/// Reads the text in a frame (an OCR plug-in). Used only to find web addresses ([`crate::urls`]); the text is dropped.
pub trait Ocr: Send + Sync {
    fn read(&self, frame: &Frame) -> Result<String, String>;
}

/// The window in front. Learning this reads no pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub app: String,
    pub title: String,
}

/// A raw image as the capture plug-in returns it (RGBA, 8 bits per channel).
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What the capture plug-in offers. `focused_window` reads no pixels. `grab` reads the pixels of that window only (never of
/// the whole screen, where another window may be showing something that must not be seen) and must fail if the window in front
/// is no longer `expect`: the user may have switched to a password manager since it was looked up.
pub trait Capture: Send + Sync {
    fn focused_window(&self) -> Result<Option<WindowInfo>, String>;
    fn grab(&self, max_edge: u32, expect: &WindowInfo) -> Result<Image, String>;
}

/// A piece of recorded speech: 16-bit mono samples.
pub struct Clip {
    pub sample_rate: u32,
    pub pcm16: Vec<i16>,
}

impl Clip {
    /// Root-mean-square level of the samples (0 for an empty clip); how loud it is, to tell speech from silence.
    pub fn rms(&self) -> u32 {
        if self.pcm16.is_empty() {
            return 0;
        }
        let sum: f64 = self.pcm16.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
        (sum / self.pcm16.len() as f64).sqrt() as u32
    }
}

/// The microphone side of the capture plug-in: what was recorded since the last call (`None` when nothing new).
pub trait Microphone: Send + Sync {
    fn read(&self) -> Result<Option<Clip>, String>;
    /// Release the recording device. Called whenever listening is not allowed (off, paused, voice switched off), so the
    /// indicator never claims less than what is happening.
    fn stop(&self) {}
}

/// Speech to text (a transcription plug-in).
pub trait Transcriber: Send + Sync {
    fn transcribe(&self, clip: &Clip) -> Result<String, String>;
}

/// Where decisions are recorded (the audit ledger). Reasons only, never screen content.
pub trait Audit: Send + Sync {
    fn record(&self, decision: &str);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub enabled: bool,
    pub capture_voice: bool,
    pub interval_ms: u64,
    pub excluded_apps: Vec<String>,
    /// frames whose 64-bit average-hash differs from the last kept frame by at most this many bits are duplicates
    pub duplicate_distance: u32,
    /// also read the text of each kept frame to find the web addresses on it (needs an OCR plug-in; off by default)
    pub extract_urls: bool,
    /// the longest side of a captured image; bigger windows are scaled down by the capture plug-in
    pub max_edge: u32,
    /// clips quieter than this (root-mean-square of 16-bit samples) are silence and never go to the transcriber
    pub speech_rms: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self { enabled: false, capture_voice: false, interval_ms: 60_000, excluded_apps: Vec::new(), duplicate_distance: 5, extract_urls: false, max_edge: 1568, speech_rms: 300 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub ts_ms: u64,
    /// "screen" or "voice"
    pub source: &'static str,
    pub app: String,
    pub text: String,
    /// web addresses read off the screen (only when the policy asks for them)
    pub urls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    Disabled,
    Paused,
    TooSoon,
    Sensitive(&'static str),
    Duplicate,
    BadFrame,
    DescribeFailed,
    NothingToReport,
    NoWindow,
    CaptureFailed,
}

impl Skip {
    pub fn reason(&self) -> String {
        match self {
            Skip::Disabled => "monitor off".into(),
            Skip::Paused => "paused".into(),
            Skip::TooSoon => "too soon after the last capture".into(),
            Skip::Sensitive(why) => format!("sensitive window: {why}"),
            Skip::Duplicate => "same as the last capture".into(),
            Skip::BadFrame => "unusable frame".into(),
            Skip::DescribeFailed => "could not describe the frame".into(),
            Skip::NothingToReport => "nothing to report".into(),
            Skip::NoWindow => "no window in front".into(),
            Skip::CaptureFailed => "could not capture the window".into(),
        }
    }
}

/// What the on-screen indicator must show. `active` is true exactly when the monitor can currently capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indicator {
    pub active: bool,
    pub paused: bool,
    pub voice: bool,
}

struct State {
    last_kept_ms: Option<u64>,
    /// When pixels were last taken (kept or not): the interval is measured from here, so an unchanged screen is not re-captured every round.
    last_capture_ms: Option<u64>,
    last_hash: Option<u64>,
    voice: VoiceBatcher,
}

pub struct Monitor {
    policy: Mutex<Policy>,
    paused: AtomicBool,
    state: Mutex<State>,
    describer: Arc<dyn Describer>,
    ocr: Option<Arc<dyn Ocr>>,
    audit: Arc<dyn Audit>,
}

/// After scrubbing, is there anything left worth remembering? "Password: [redacted]" is not.
fn has_substance(text: &str) -> bool {
    const LABELS: &[&str] = &["password", "passwd", "passcode", "pin", "secret", "token", "key", "cvv", "cvc", "ssn", "api", "security", "code"];
    text.replace("[redacted]", " ").split(|c: char| !c.is_alphabetic()).any(|w| w.chars().count() >= 3 && !LABELS.contains(&w.to_lowercase().as_str()))
}

/// 64-bit average hash: the frame is reduced to 8x8 blocks of brightness, each bit says "brighter than average".
pub fn average_hash(f: &Frame) -> Option<u64> {
    let (w, h) = (f.width as usize, f.height as usize);
    if w == 0 || h == 0 || f.width > MAX_DIMENSION || f.height > MAX_DIMENSION || f.rgba.len() != w.checked_mul(h)?.checked_mul(4)? {
        return None;
    }
    let mut sums = [0u64; 64];
    let mut counts = [0u64; 64];
    for y in 0..h {
        let by = y * 8 / h;
        for x in 0..w {
            let bx = x * 8 / w;
            let i = (y * w + x) * 4;
            // integer luma approximation
            let l = (u64::from(f.rgba[i]) * 299 + u64::from(f.rgba[i + 1]) * 587 + u64::from(f.rgba[i + 2]) * 114) / 1000;
            sums[by * 8 + bx] += l;
            counts[by * 8 + bx] += 1;
        }
    }
    let avg: Vec<u64> = sums.iter().zip(&counts).map(|(s, c)| if *c == 0 { 0 } else { s / c }).collect();
    let mean = avg.iter().sum::<u64>() / 64;
    Some(avg.iter().enumerate().fold(0u64, |acc, (i, a)| if *a > mean { acc | (1 << i) } else { acc }))
}

impl Monitor {
    pub fn new(policy: Policy, describer: Arc<dyn Describer>, audit: Arc<dyn Audit>) -> Self {
        Self { policy: Mutex::new(policy), paused: AtomicBool::new(false), state: Mutex::new(State { last_kept_ms: None, last_capture_ms: None, last_hash: None, voice: VoiceBatcher::default() }), describer, ocr: None, audit }
    }

    /// Give the monitor an OCR engine, used only when the policy has `extract_urls` on.
    pub fn with_ocr(mut self, ocr: Arc<dyn Ocr>) -> Self {
        self.ocr = Some(ocr);
        self
    }

    pub fn set_policy(&self, p: Policy) {
        *self.policy.lock().unwrap() = p;
    }

    /// Stop capturing immediately. Takes effect before the next frame is looked at.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
        self.audit.record("monitor paused");
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
        self.audit.record("monitor resumed");
    }

    pub fn indicator(&self) -> Indicator {
        let p = self.policy.lock().unwrap();
        let paused = self.paused.load(Ordering::SeqCst);
        Indicator { active: p.enabled && !paused, paused, voice: p.enabled && p.capture_voice && !paused }
    }

    fn skip(&self, s: Skip) -> Result<Observation, Skip> {
        self.audit.record(&format!("skipped: {}", s.reason()));
        Err(s)
    }

    pub fn process(&self, frame: Frame) -> Result<Observation, Skip> {
        let policy = self.policy.lock().unwrap().clone();
        if !policy.enabled {
            return Err(Skip::Disabled); // not audited per frame: a disabled monitor is silent
        }
        if self.paused.load(Ordering::SeqCst) {
            return Err(Skip::Paused);
        }
        {
            let st = self.state.lock().unwrap();
            if let Some(last) = st.last_kept_ms {
                if frame.ts_ms.saturating_sub(last) < policy.interval_ms {
                    return Err(Skip::TooSoon);
                }
            }
        }
        if let Some(why) = sensitive_window(&frame.app, &frame.title, &policy.excluded_apps) {
            return self.skip(Skip::Sensitive(why));
        }
        let Some(hash) = average_hash(&frame) else { return self.skip(Skip::BadFrame) };
        {
            let st = self.state.lock().unwrap();
            if st.last_hash.is_some_and(|h| (h ^ hash).count_ones() <= policy.duplicate_distance) {
                return self.skip(Skip::Duplicate);
            }
        }
        let described = self.describer.describe(&frame);
        // the addresses on the screen, if asked for: the OCR text itself is dropped here with the pixels
        let urls = match (&self.ocr, policy.extract_urls, &described) {
            (Some(ocr), true, Ok(_)) => ocr.read(&frame).map(|t| crate::urls::extract_urls(&t)).unwrap_or_default(),
            _ => Vec::new(),
        };
        let app = frame.app.clone();
        let ts_ms = frame.ts_ms;
        drop(frame); // the pixels are gone from here on
        let Ok(raw) = described else { return self.skip(Skip::DescribeFailed) };
        let text: String = redact(raw.trim()).chars().take(MAX_TEXT_CHARS).collect();
        if !has_substance(&text) {
            return self.skip(Skip::NothingToReport);
        }
        {
            let mut st = self.state.lock().unwrap();
            st.last_kept_ms = Some(ts_ms);
            st.last_hash = Some(hash);
        }
        self.audit.record("kept: screen observation");
        Ok(Observation { ts_ms, source: "screen", app, text, urls })
    }

    /// One capture round with a capture plug-in, in the order that protects the user: monitor on? paused? too soon? which window
    /// is in front (no pixels read)? sensitive? and only then the pixels of that window. A sensitive window is never captured.
    pub fn tick(&self, cap: &dyn Capture, now_ms: u64) -> Result<Observation, Skip> {
        let policy = self.policy.lock().unwrap().clone();
        if !policy.enabled {
            return Err(Skip::Disabled);
        }
        if self.paused.load(Ordering::SeqCst) {
            return Err(Skip::Paused);
        }
        {
            let st = self.state.lock().unwrap();
            if let Some(last) = st.last_kept_ms.max(st.last_capture_ms) {
                if now_ms.saturating_sub(last) < policy.interval_ms {
                    return Err(Skip::TooSoon);
                }
            }
        }
        let win = match cap.focused_window() {
            Ok(Some(w)) => w,
            Ok(None) => return self.skip(Skip::NoWindow),
            Err(_) => return self.skip(Skip::CaptureFailed),
        };
        if let Some(why) = sensitive_window(&win.app, &win.title, &policy.excluded_apps) {
            return self.skip(Skip::Sensitive(why));
        }
        // small print does not survive being scaled down: reading addresses needs more pixels than describing does
        let edge = if policy.extract_urls { policy.max_edge.max(MIN_EDGE_FOR_OCR) } else { policy.max_edge };
        self.state.lock().unwrap().last_capture_ms = Some(now_ms);
        let Ok(img) = cap.grab(edge, &win) else { return self.skip(Skip::CaptureFailed) };
        self.process(Frame { ts_ms: now_ms, width: img.width, height: img.height, rgba: img.rgba, app: win.app, title: win.title })
    }

    /// One listening round: read what the microphone recorded, drop silence without transcribing it, transcribe speech, and hand
    /// back an utterance when the speaker has paused or said enough. Nothing is read from the microphone while the monitor is off,
    /// paused, or has voice switched off.
    pub fn listen(&self, mic: &dyn Microphone, stt: &dyn Transcriber, now_ms: u64) -> Option<Observation> {
        let policy = self.policy.lock().unwrap().clone();
        if !policy.enabled || !policy.capture_voice || self.paused.load(Ordering::SeqCst) {
            mic.stop();
            return None;
        }
        let clip = match mic.read() {
            Ok(Some(c)) => c,
            Ok(None) => return self.idle(now_ms),
            Err(_) => {
                self.audit.record("skipped: could not read the microphone");
                return None;
            }
        };
        if clip.rms() < policy.speech_rms {
            return self.idle(now_ms);
        }
        match stt.transcribe(&clip) {
            Ok(text) => self.voice(now_ms, &text),
            Err(_) => {
                self.audit.record("skipped: could not transcribe speech");
                None
            }
        }
    }

    /// Nothing was said this round: a long enough silence closes the utterance being gathered.
    fn idle(&self, now_ms: u64) -> Option<Observation> {
        let out = self.state.lock().unwrap().voice.idle(now_ms);
        out.and_then(|u| self.finish_utterance(u))
    }

    /// A transcribed piece of speech. Returns an utterance when the speaker paused long enough or enough was said.
    pub fn voice(&self, ts_ms: u64, text: &str) -> Option<Observation> {
        let policy = self.policy.lock().unwrap().clone();
        if !policy.enabled || !policy.capture_voice || self.paused.load(Ordering::SeqCst) {
            return None;
        }
        let out = self.state.lock().unwrap().voice.push(ts_ms, text);
        out.and_then(|u| self.finish_utterance(u))
    }

    /// Emit whatever speech is still waiting (end of session, pause, shutdown).
    pub fn flush_voice(&self) -> Option<Observation> {
        let u = self.state.lock().unwrap().voice.flush();
        u.and_then(|u| self.finish_utterance(u))
    }

    fn finish_utterance(&self, u: (u64, String)) -> Option<Observation> {
        let text = redact(u.1.trim());
        if !has_substance(&text) {
            return None;
        }
        self.audit.record("kept: voice observation");
        Some(Observation { ts_ms: u.0, source: "voice", app: String::new(), text, urls: Vec::new() })
    }
}

/// Joins consecutive transcript chunks into utterances: a gap of silence ends one, and so does reaching the size cap.
pub struct VoiceBatcher {
    pub gap_ms: u64,
    pub max_chars: usize,
    start_ms: u64,
    last_ms: u64,
    text: String,
}

impl Default for VoiceBatcher {
    fn default() -> Self {
        Self { gap_ms: 3_000, max_chars: 2_000, start_ms: 0, last_ms: 0, text: String::new() }
    }
}

impl VoiceBatcher {
    /// Add a chunk; returns the finished utterance (start time, text) if this chunk closed one.
    pub fn push(&mut self, ts_ms: u64, chunk: &str) -> Option<(u64, String)> {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            return None;
        }
        let mut done = None;
        if !self.text.is_empty() && ts_ms.saturating_sub(self.last_ms) > self.gap_ms {
            done = self.flush();
        }
        if self.text.is_empty() {
            self.start_ms = ts_ms;
        } else {
            self.text.push(' ');
        }
        self.text.push_str(chunk);
        self.last_ms = ts_ms;
        if self.text.chars().count() >= self.max_chars {
            // a size-capped utterance closes now; if a gap already closed one, hand that one out first and keep this
            return match done {
                Some(d) => Some(d),
                None => self.flush(),
            };
        }
        done
    }

    /// Silence at `ts_ms`: closes the utterance if the last words were more than `gap_ms` ago.
    pub fn idle(&mut self, ts_ms: u64) -> Option<(u64, String)> {
        if !self.text.is_empty() && ts_ms.saturating_sub(self.last_ms) > self.gap_ms {
            return self.flush();
        }
        None
    }

    pub fn flush(&mut self) -> Option<(u64, String)> {
        if self.text.is_empty() {
            return None;
        }
        Some((self.start_ms, std::mem::take(&mut self.text)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake(Mutex<Vec<Result<String, String>>>);
    impl Describer for Fake {
        fn describe(&self, _: &Frame) -> Result<String, String> {
            self.0.lock().unwrap().pop().unwrap_or(Err("no more".into()))
        }
    }

    #[derive(Default)]
    struct Log(Mutex<Vec<String>>);
    impl Audit for Log {
        fn record(&self, d: &str) {
            self.0.lock().unwrap().push(d.to_string());
        }
    }

    fn frame(ts: u64, pattern: u8, app: &str, title: &str) -> Frame {
        // 16x16 image: left half bright or right half bright, so different patterns hash very differently
        let (w, h) = (16u32, 16u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let bright = match pattern {
                    0 => x < 8,
                    1 => x >= 8,
                    _ => y < 8,
                };
                let v = if bright { 250 } else { 5 };
                let i = ((y * w + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        Frame { ts_ms: ts, width: w, height: h, rgba, app: app.into(), title: title.into() }
    }

    fn setup(replies: &[&str], policy: Policy) -> (Monitor, Arc<Log>) {
        let log = Arc::new(Log::default());
        let mut v: Vec<Result<String, String>> = replies.iter().map(|r| Ok((*r).to_string())).collect();
        v.reverse();
        (Monitor::new(policy, Arc::new(Fake(Mutex::new(v))), log.clone()), log)
    }

    fn on() -> Policy {
        Policy { enabled: true, capture_voice: true, interval_ms: 1000, ..Policy::default() }
    }

    #[test]
    fn it_is_off_by_default_and_the_indicator_tells_the_truth() {
        let (m, log) = setup(&["x"], Policy::default());
        assert_eq!(m.process(frame(0, 0, "Code", "main.rs")).unwrap_err(), Skip::Disabled);
        assert_eq!(m.indicator(), Indicator { active: false, paused: false, voice: false });
        assert!(log.0.lock().unwrap().is_empty(), "a disabled monitor does not even log");
        m.set_policy(on());
        assert_eq!(m.indicator(), Indicator { active: true, paused: false, voice: true });
        m.pause();
        assert_eq!(m.indicator(), Indicator { active: false, paused: true, voice: false });
        assert_eq!(m.process(frame(5000, 0, "Code", "main.rs")).unwrap_err(), Skip::Paused);
        m.resume();
        assert!(m.indicator().active);
    }

    #[test]
    fn a_frame_becomes_scrubbed_text_and_only_a_reason_is_audited() {
        let (m, log) = setup(&["The user edits main.rs. A terminal shows Password: hunter2 and key sk-abcdefghijklmnop1234."], on());
        let o = m.process(frame(10_000, 0, "Code", "main.rs")).unwrap();
        assert_eq!((o.source, o.app.as_str(), o.ts_ms), ("screen", "Code", 10_000));
        assert!(o.text.contains("edits main.rs") && !o.text.contains("hunter2") && !o.text.contains("sk-abc"), "{}", o.text);
        let entries = log.0.lock().unwrap().join("|");
        assert_eq!(entries, "kept: screen observation");
        assert!(!entries.contains("main.rs"));
    }

    #[test]
    fn sensitive_windows_are_skipped_before_the_describer_is_ever_called() {
        let (m, log) = setup(&[], on()); // an empty script would make a describe call fail: that would show as DescribeFailed
        for (app, title) in [("1Password", "Vault"), ("chrome", "Online Banking"), ("Code", "x")] {
            let r = m.process(frame(10_000, 0, app, title));
            if app == "Code" {
                assert_eq!(r.unwrap_err(), Skip::DescribeFailed);
            } else {
                assert!(matches!(r.unwrap_err(), Skip::Sensitive(_)), "{app}");
            }
        }
        assert!(log.0.lock().unwrap().iter().filter(|l| l.starts_with("skipped: sensitive")).count() == 2);
        let (m, _) = setup(&[], Policy { excluded_apps: vec!["discord".into()], ..on() });
        assert!(matches!(m.process(frame(10_000, 0, "Discord", "chat")).unwrap_err(), Skip::Sensitive("excluded app")));
    }

    #[test]
    fn near_duplicates_and_rapid_frames_are_skipped_but_real_changes_are_kept() {
        let (m, _) = setup(&["first", "second"], on());
        assert!(m.process(frame(10_000, 0, "Code", "a")).is_ok());
        assert_eq!(m.process(frame(10_200, 1, "Code", "a")).unwrap_err(), Skip::TooSoon);
        assert_eq!(m.process(frame(20_000, 0, "Code", "a")).unwrap_err(), Skip::Duplicate);
        assert_eq!(m.process(frame(30_000, 1, "Code", "a")).unwrap().text, "second");
    }

    #[test]
    fn failures_and_empty_results_are_skips_not_errors_and_do_not_move_the_baseline() {
        let (m, _) = setup(&["", "Password: x", "real"], on());
        assert_eq!(m.process(frame(10_000, 0, "Code", "a")).unwrap_err(), Skip::NothingToReport);
        assert_eq!(m.process(frame(20_000, 0, "Code", "a")).unwrap_err(), Skip::NothingToReport, "fully redacted text is dropped");
        assert_eq!(m.process(frame(30_000, 0, "Code", "a")).unwrap().text, "real", "the baseline was not set by skipped frames");
        let (m, _) = setup(&[], on());
        let mut bad = frame(10_000, 0, "Code", "a");
        bad.rgba.truncate(10);
        assert_eq!(m.process(bad).unwrap_err(), Skip::BadFrame);
        let mut huge = frame(10_000, 0, "Code", "a");
        huge.width = 100_000;
        assert_eq!(m.process(huge).unwrap_err(), Skip::BadFrame);
    }

    #[test]
    fn voice_chunks_are_batched_by_silence_and_scrubbed() {
        let (m, _) = setup(&[], on());
        assert!(m.voice(1000, "remind me").is_none());
        assert!(m.voice(2000, "to call mom").is_none());
        let u = m.voice(9000, "new topic").expect("a gap closes the first utterance");
        assert_eq!((u.source, u.ts_ms, u.text.as_str()), ("voice", 1000, "remind me to call mom"));
        let last = m.flush_voice().unwrap();
        assert_eq!(last.text, "new topic");
        assert!(m.flush_voice().is_none());
        assert!(m.voice(20_000, "my password is hunter2").is_none());
        assert!(m.flush_voice().is_none(), "an utterance that is only a secret is dropped entirely");
        assert!(m.voice(30_000, "the meeting is at noon, my pin is 4821").is_none());
        assert_eq!(m.flush_voice().unwrap().text, "the meeting is at noon, my pin: [redacted]");
    }

    #[test]
    fn voice_respects_the_switches_and_the_size_cap() {
        let (m, _) = setup(&[], Policy { capture_voice: false, ..on() });
        assert!(m.voice(1000, "hello").is_none() && m.flush_voice().is_none(), "voice capture is a separate opt-in");
        let mut b = VoiceBatcher { max_chars: 20, ..VoiceBatcher::default() };
        assert!(b.push(0, "twelve chars").is_none());
        assert_eq!(b.push(100, "and more words").unwrap().1, "twelve chars and more words");
        assert!(b.flush().is_none());
        assert!(b.push(0, "   ").is_none());
    }


    // ---------------------------------------------------------------- the capture plug-in's side
    use std::sync::atomic::AtomicUsize;

    /// A screen with a script of "who is in front" answers; counts how many times pixels were asked for.
    struct Screen {
        fronts: Mutex<Vec<Option<WindowInfo>>>,
        grabs: AtomicUsize,
        pattern: u8,
    }

    impl Screen {
        fn new(fronts: &[(&str, &str)], pattern: u8) -> Self {
            let mut v: Vec<Option<WindowInfo>> = fronts.iter().map(|(a, t)| Some(WindowInfo { app: (*a).into(), title: (*t).into() })).collect();
            v.reverse();
            Self { fronts: Mutex::new(v), grabs: AtomicUsize::new(0), pattern }
        }
    }

    impl Capture for Screen {
        fn focused_window(&self) -> Result<Option<WindowInfo>, String> {
            Ok(self.fronts.lock().unwrap().last().cloned().flatten())
        }
        fn grab(&self, _max_edge: u32, expect: &WindowInfo) -> Result<Image, String> {
            self.grabs.fetch_add(1, Ordering::SeqCst);
            let now = self.focused_window()?;
            if now.as_ref() != Some(expect) {
                return Err("focus changed".into());
            }
            let f = frame(0, self.pattern, "", "");
            Ok(Image { width: f.width, height: f.height, rgba: f.rgba })
        }
    }

    #[test]
    fn a_sensitive_window_is_never_grabbed() {
        let (m, log) = setup(&[], on());
        let screen = Screen::new(&[("1Password", "Vault"), ("chrome", "Online Banking - Sign in")], 0);
        assert!(matches!(m.tick(&screen, 10_000).unwrap_err(), Skip::Sensitive(_)));
        screen.fronts.lock().unwrap().pop(); // the next round sees the banking window
        assert!(matches!(m.tick(&screen, 20_000).unwrap_err(), Skip::Sensitive(_)));
        assert_eq!(screen.grabs.load(Ordering::SeqCst), 0, "no pixel was read for either window");
        assert_eq!(log.0.lock().unwrap().iter().filter(|l| l.starts_with("skipped: sensitive")).count(), 2);
    }

    #[test]
    fn switching_to_another_window_between_the_look_and_the_grab_fails_the_grab() {
        let (m, log) = setup(&["a text editor"], on());
        // the capture plug-in's focus answer changes after the first lookup (the user alt-tabbed to a password manager)
        struct Switcher(Screen, AtomicUsize);
        impl Capture for Switcher {
            fn focused_window(&self) -> Result<Option<WindowInfo>, String> {
                let n = self.1.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Ok(Some(WindowInfo { app: "Code".into(), title: "main.rs".into() }))
                } else {
                    Ok(Some(WindowInfo { app: "KeePass".into(), title: "Database".into() }))
                }
            }
            fn grab(&self, max_edge: u32, expect: &WindowInfo) -> Result<Image, String> {
                self.0.grabs.fetch_add(1, Ordering::SeqCst);
                if self.focused_window()?.as_ref() != Some(expect) {
                    return Err("focus changed".into());
                }
                self.0.grab(max_edge, expect)
            }
        }
        let sw = Switcher(Screen::new(&[("Code", "main.rs")], 0), AtomicUsize::new(0));
        assert_eq!(m.tick(&sw, 10_000).unwrap_err(), Skip::CaptureFailed);
        assert!(log.0.lock().unwrap().iter().any(|l| l.contains("could not capture")));
    }

    #[test]
    fn an_ordinary_window_is_grabbed_described_and_kept_and_the_rounds_obey_the_switches() {
        let (m, _) = setup(&["a text editor with Rust code"], on());
        let screen = Screen::new(&[("Code", "main.rs")], 0);
        let o = m.tick(&screen, 10_000).unwrap();
        assert_eq!((o.source, o.app.as_str(), o.text.as_str()), ("screen", "Code", "a text editor with Rust code"));
        assert_eq!(screen.grabs.load(Ordering::SeqCst), 1);
        assert_eq!(m.tick(&screen, 10_500).unwrap_err(), Skip::TooSoon, "within the interval nothing is even looked at");
        assert_eq!(screen.grabs.load(Ordering::SeqCst), 1);
        m.pause();
        assert_eq!(m.tick(&screen, 60_000).unwrap_err(), Skip::Paused);
        m.resume();
        m.set_policy(Policy { enabled: false, ..on() });
        assert_eq!(m.tick(&screen, 60_000).unwrap_err(), Skip::Disabled);
        assert_eq!(screen.grabs.load(Ordering::SeqCst), 1, "off and paused read nothing");
        // no window in front at all
        m.set_policy(on());
        let empty = Screen { fronts: Mutex::new(vec![None]), grabs: AtomicUsize::new(0), pattern: 0 };
        assert_eq!(m.tick(&empty, 90_000).unwrap_err(), Skip::NoWindow);
    }

    #[test]
    fn an_unchanged_screen_is_not_captured_again_before_the_interval_is_up() {
        let (m, _) = setup(&["a text editor with Rust code"], on());
        let screen = Screen::new(&[("Code", "main.rs")], 0);
        assert!(m.tick(&screen, 10_000).is_ok());
        assert_eq!(m.tick(&screen, 20_000).unwrap_err(), Skip::Duplicate, "the same picture is dropped");
        assert_eq!(screen.grabs.load(Ordering::SeqCst), 2);
        assert_eq!(m.tick(&screen, 20_500).unwrap_err(), Skip::TooSoon, "the interval counts from the last capture, kept or not");
        assert_eq!(screen.grabs.load(Ordering::SeqCst), 2, "no pixels are read again within the interval");
    }

    struct Ocr1(String);
    impl Ocr for Ocr1 {
        fn read(&self, _: &Frame) -> Result<String, String> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn addresses_are_read_only_when_asked_for_and_the_ocr_text_is_not_kept() {
        let text = "Docs at https://example.com/guide and github.com/user/repo, see e.g. the footer";
        for (ask, want) in [(false, 0usize), (true, 2)] {
            let log = Arc::new(Log::default());
            let m = Monitor::new(Policy { extract_urls: ask, ..on() }, Arc::new(Fake(Mutex::new(vec![Ok("a web page about docs".into())]))), log).with_ocr(Arc::new(Ocr1(text.into())));
            let o = m.process(frame(10_000, 0, "Chrome", "Docs")).unwrap();
            assert_eq!(o.urls.len(), want, "extract_urls = {ask}");
            assert_eq!(o.text, "a web page about docs", "the OCR text is not what is remembered");
            if ask {
                // as upstream finds them: the comma after the second belongs to its path pattern; `urls::tidy` removes it for use
                assert_eq!(o.urls.iter().map(|u| crate::urls::tidy(u)).collect::<Vec<_>>(), ["https://example.com/guide", "https://github.com/user/repo"]);
            }
        }
        // no OCR plug-in installed: still works, no addresses
        let (m, _) = setup(&["a page"], Policy { extract_urls: true, ..on() });
        assert!(m.process(frame(10_000, 0, "Chrome", "x")).unwrap().urls.is_empty());
    }

    // ---------------------------------------------------------------- the microphone
    struct Mic(Mutex<Vec<Option<Clip>>>, AtomicUsize, AtomicUsize);
    impl Microphone for Mic {
        fn read(&self) -> Result<Option<Clip>, String> {
            self.1.fetch_add(1, Ordering::SeqCst);
            Ok(self.0.lock().unwrap().pop().flatten())
        }
        fn stop(&self) {
            self.2.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct Stt(Mutex<Vec<String>>, AtomicUsize);
    impl Transcriber for Stt {
        fn transcribe(&self, _: &Clip) -> Result<String, String> {
            self.1.fetch_add(1, Ordering::SeqCst);
            self.0.lock().unwrap().pop().ok_or_else(|| "no more".to_string())
        }
    }

    fn clip(level: i16) -> Clip {
        Clip { sample_rate: 16_000, pcm16: (0..1600).map(|i| if i % 2 == 0 { level } else { -level }).collect() }
    }

    #[test]
    fn silence_is_never_transcribed_speech_is_batched_and_the_switches_are_respected() {
        assert_eq!(clip(0).rms(), 0);
        assert_eq!(clip(1000).rms(), 1000);
        let (m, _) = setup(&[], on());
        // newest last in the script, popped from the end: speech, speech, silence, silence-after-a-gap
        let mic = Mic(Mutex::new(vec![None, Some(clip(10)), Some(clip(2000)), Some(clip(2000))]), AtomicUsize::new(0), AtomicUsize::new(0));
        let stt = Stt(Mutex::new(vec!["at five".into(), "call mom".into()]), AtomicUsize::new(0));
        assert!(m.listen(&mic, &stt, 1_000).is_none(), "first words start an utterance");
        assert!(m.listen(&mic, &stt, 2_000).is_none(), "more words join it");
        assert!(m.listen(&mic, &stt, 3_000).is_none(), "a quiet clip is silence, and 1 s of it does not end the utterance");
        let u = m.listen(&mic, &stt, 9_000).expect("after a long silence the utterance closes");
        assert_eq!((u.source, u.text.as_str()), ("voice", "call mom at five"));
        assert_eq!(stt.1.load(Ordering::SeqCst), 2, "only the two loud clips were transcribed");
        // voice off, monitor paused, monitor off: the microphone is not even read
        let reads = mic.1.load(Ordering::SeqCst);
        m.set_policy(Policy { capture_voice: false, ..on() });
        assert!(m.listen(&mic, &stt, 20_000).is_none());
        m.set_policy(on());
        m.pause();
        assert!(m.listen(&mic, &stt, 20_000).is_none());
        m.resume();
        m.set_policy(Policy::default());
        assert!(m.listen(&mic, &stt, 20_000).is_none());
        assert_eq!(mic.1.load(Ordering::SeqCst), reads, "nothing was read while it must not listen");
        assert_eq!(mic.2.load(Ordering::SeqCst), 3, "and the device was released each time (voice off, paused, monitor off)");
    }

    #[test]
    fn spoken_secrets_are_scrubbed_like_screen_text() {
        let (m, _) = setup(&[], on());
        let mic = Mic(Mutex::new(vec![None, Some(clip(2000))]), AtomicUsize::new(0), AtomicUsize::new(0));
        let stt = Stt(Mutex::new(vec!["I like green tea and my password: hunter2hunter2".into()]), AtomicUsize::new(0));
        assert!(m.listen(&mic, &stt, 1_000).is_none());
        let u = m.listen(&mic, &stt, 10_000).unwrap();
        assert!(!u.text.contains("hunter2"), "{}", u.text);
        assert!(u.text.contains("green tea"));
    }

    #[test]
    fn the_hash_separates_different_screens_and_rejects_malformed_frames() {
        let a = average_hash(&frame(0, 0, "", "")).unwrap();
        let b = average_hash(&frame(0, 1, "", "")).unwrap();
        let c = average_hash(&frame(0, 0, "", "")).unwrap();
        assert_eq!(a, c);
        assert!((a ^ b).count_ones() > 20);
        let mut f = frame(0, 0, "", "");
        f.height = 0;
        assert!(average_hash(&f).is_none());
    }
}
