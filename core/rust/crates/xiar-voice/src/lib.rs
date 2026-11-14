// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Voices: how a bot speaks. One trait for every engine, so a bot's voice is "this engine, this voice" and any engine can be added by a
//! plug-in.
//!
//! * **`system`** is the default: the operating system's own voices (on Windows every installed SAPI voice, IVONA ones included). Nothing
//!   is bundled and nothing leaves the computer.
//! * **`chatterbox`** clones a voice from a reference clip (such as `Emma.wav`) through the container that is already on this computer.
//! * The person chooses the engine per bot; a choice that names an engine or voice that is not there is refused with the reason.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

pub const SYSTEM: &str = "system";
pub const CHATTERBOX: &str = "chatterbox";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceInfo {
    pub engine: String,
    /// what the engine calls it (a SAPI token id, a clip's file name)
    pub id: String,
    pub name: String,
    /// a language tag such as `en-GB`, when known
    pub language: String,
    /// `female`, `male` or empty
    pub gender: String,
}

/// How a bot speaks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceChoice {
    pub engine: String,
    /// empty = the engine's default voice
    #[serde(default)]
    pub voice_id: String,
    /// a clip to clone, for engines that clone (a full path)
    #[serde(default)]
    pub reference_clip: Option<PathBuf>,
    /// 0.5 (slow) to 2.0 (fast)
    #[serde(default = "one")]
    pub rate: f32,
    /// 0.0 to 1.0
    #[serde(default = "one")]
    pub volume: f32,
}

fn one() -> f32 {
    1.0
}

impl Default for VoiceChoice {
    /// The system voice is the default.
    fn default() -> Self {
        Self { engine: SYSTEM.into(), voice_id: String::new(), reference_clip: None, rate: 1.0, volume: 1.0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spoken {
    /// a WAV file's bytes
    pub wav: Vec<u8>,
    pub ms: u64,
}

pub trait VoiceEngine: Send + Sync {
    fn id(&self) -> &str;
    fn label(&self) -> &str;
    /// Whether it can speak right now; the reason when it can not (a container that is not running, a plug-in that is not installed).
    fn availability(&self) -> Result<(), String>;
    fn voices(&self) -> Result<Vec<VoiceInfo>, String>;
    fn speak(&self, text: &str, choice: &VoiceChoice) -> Result<Spoken, String>;
    /// Does the engine clone voices from a clip?
    fn clones(&self) -> bool {
        false
    }
    /// Does it take any voice name the person types (it has no list of its own)?
    fn free_voice_ids(&self) -> bool {
        false
    }
}

/// Something that turns speech into text (an engine on this computer, a program, a cloud service). The counterpart of [`VoiceEngine`].
pub trait Listener: Send + Sync {
    fn id(&self) -> &str;
    fn label(&self) -> &str;
    /// Whether it can listen right now; the reason when it can not.
    fn availability(&self) -> Result<(), String>;
    /// The words in a WAV recording. `language` is a tag such as `en`, or empty for "work it out".
    fn transcribe(&self, wav: &[u8], language: &str) -> Result<String, String>;
}

#[derive(Default)]
pub struct VoiceRegistry {
    engines: Vec<Arc<dyn VoiceEngine>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EngineView {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub why_not: Option<String>,
    pub clones: bool,
    pub voices: Vec<VoiceInfo>,
}

impl VoiceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, e: Arc<dyn VoiceEngine>) {
        self.engines.retain(|x| x.id() != e.id());
        self.engines.push(e);
    }

    pub fn engine(&self, id: &str) -> Option<&Arc<dyn VoiceEngine>> {
        self.engines.iter().find(|e| e.id() == id)
    }

    /// Every engine with whether it works and the voices it offers. The system engine comes first.
    pub fn overview(&self) -> Vec<EngineView> {
        let mut v: Vec<EngineView> = self
            .engines
            .iter()
            .map(|e| {
                let avail = e.availability();
                let voices = if avail.is_ok() { e.voices().unwrap_or_default() } else { vec![] };
                EngineView { id: e.id().into(), label: e.label().into(), available: avail.is_ok(), why_not: avail.err(), clones: e.clones(), voices }
            })
            .collect();
        v.sort_by_key(|e| (e.id != SYSTEM, e.id.clone()));
        v
    }

    /// Is this choice something a bot can be set to? Says why not.
    pub fn check(&self, c: &VoiceChoice) -> Result<(), String> {
        if !(0.5..=2.0).contains(&c.rate) || !(0.0..=1.0).contains(&c.volume) {
            return Err("the speed is 0.5 to 2.0 and the volume 0 to 1".into());
        }
        let e = self.engine(&c.engine).ok_or_else(|| format!("there is no voice engine {:?}", c.engine))?;
        e.availability().map_err(|why| format!("{} is not available: {why}", e.label()))?;
        if e.clones() {
            let has_clip = c.reference_clip.as_ref().is_some_and(|p| p.is_absolute() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav") || x.eq_ignore_ascii_case("mp3")));
            if !has_clip && c.voice_id.is_empty() {
                return Err(format!("{} speaks in a cloned voice: choose a reference clip (a .wav or .mp3 file, full path) or one of its voices", e.label()));
            }
        }
        if !c.voice_id.is_empty() && !e.clones() && !e.free_voice_ids() && !e.voices()?.iter().any(|v| v.id == c.voice_id) {
            return Err(format!("{} does not have the voice {:?}", e.label(), c.voice_id));
        }
        Ok(())
    }

    /// Speaks as `choice` says; falls back to the system voice, and says so, when the chosen engine fails.
    pub fn speak(&self, text: &str, choice: &VoiceChoice) -> Result<(Spoken, Option<String>), String> {
        if text.trim().is_empty() {
            return Err("there is nothing to say".into());
        }
        let first = self.engine(&choice.engine).ok_or_else(|| format!("there is no voice engine {:?}", choice.engine)).and_then(|e| e.speak(text, choice));
        match first {
            Ok(s) => Ok((s, None)),
            Err(why) if choice.engine != SYSTEM => {
                let sys = self.engine(SYSTEM).ok_or(why.clone())?;
                let s = sys.speak(text, &VoiceChoice { rate: choice.rate, volume: choice.volume, ..VoiceChoice::default() })?;
                Ok((s, Some(format!("{} could not speak ({why}), so the system voice was used", choice.engine))))
            }
            Err(why) => Err(why),
        }
    }
}

/// Checks that bytes are a WAV file with some sound in it (not silence), and returns its duration in milliseconds.
pub fn wav_info(wav: &[u8]) -> Result<(u64, bool), String> {
    if wav.len() < 44 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let (mut pos, mut rate, mut channels, mut bits) = (12usize, 0u32, 0u16, 0u16);
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= wav.len() {
        let id = &wav[pos..pos + 4];
        let size = u32::from_le_bytes([wav[pos + 4], wav[pos + 5], wav[pos + 6], wav[pos + 7]]) as usize;
        let body = &wav[pos + 8..(pos + 8 + size).min(wav.len())];
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            data = Some(body);
            break;
        }
        pos += 8 + size + (size & 1);
    }
    let data = data.ok_or("no sound data in the WAV file")?;
    if rate == 0 || channels == 0 || bits != 16 {
        return Err("only 16-bit WAV files are understood".into());
    }
    let samples = data.len() / 2;
    let loud = data.as_chunks::<2>().0.iter().any(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs() > 300);
    Ok(((samples as u64 / u64::from(channels)) * 1000 / u64::from(rate), loud))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV of a short beep (or silence).
    pub fn wav(loud: bool, ms: u32) -> Vec<u8> {
        let rate = 16_000u32;
        let n = (rate * ms / 1000) as usize;
        let mut data = Vec::new();
        for i in 0..n {
            let s = if loud { ((i as f32 / 20.0).sin() * 8000.0) as i16 } else { 0 };
            data.extend_from_slice(&s.to_le_bytes());
        }
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&[1, 0, 1, 0]);
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * 2).to_le_bytes());
        v.extend_from_slice(&[2, 0, 16, 0]);
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(&data);
        v
    }

    struct Fake {
        id: &'static str,
        ok: bool,
        clones: bool,
    }

    impl VoiceEngine for Fake {
        fn id(&self) -> &str {
            self.id
        }
        fn label(&self) -> &str {
            self.id
        }
        fn availability(&self) -> Result<(), String> {
            if self.ok { Ok(()) } else { Err("the container is not running".into()) }
        }
        fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
            Ok(vec![VoiceInfo { engine: self.id.into(), id: "emma-token".into(), name: "IVONA 2 Emma".into(), language: "en-GB".into(), gender: "female".into() }])
        }
        fn speak(&self, _: &str, _: &VoiceChoice) -> Result<Spoken, String> {
            if self.ok { Ok(Spoken { wav: wav(true, 500), ms: 500 }) } else { Err("no connection".into()) }
        }
        fn clones(&self) -> bool {
            self.clones
        }
    }

    fn registry(cb_ok: bool) -> VoiceRegistry {
        let mut r = VoiceRegistry::new();
        r.add(Arc::new(Fake { id: CHATTERBOX, ok: cb_ok, clones: true }));
        r.add(Arc::new(Fake { id: SYSTEM, ok: true, clones: false }));
        r
    }

    #[test]
    fn the_system_voice_is_the_default_and_comes_first() {
        let r = registry(true);
        assert_eq!(VoiceChoice::default().engine, SYSTEM);
        assert_eq!(r.overview().iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), [SYSTEM, CHATTERBOX]);
        r.check(&VoiceChoice::default()).unwrap();
    }

    #[test]
    fn a_choice_must_name_an_engine_and_a_voice_that_exist() {
        let r = registry(true);
        let sys = |id: &str| VoiceChoice { voice_id: id.into(), ..VoiceChoice::default() };
        r.check(&sys("emma-token")).unwrap();
        assert!(r.check(&sys("nobody")).unwrap_err().contains("does not have the voice"));
        assert!(r.check(&VoiceChoice { engine: "magic".into(), ..VoiceChoice::default() }).unwrap_err().contains("no voice engine"));
        assert!(r.check(&VoiceChoice { rate: 3.0, ..VoiceChoice::default() }).is_err());
        assert!(r.check(&VoiceChoice { volume: -1.0, ..VoiceChoice::default() }).is_err());
    }

    #[test]
    fn a_cloning_engine_needs_a_reference_clip_with_a_full_path() {
        let r = registry(true);
        let cb = |clip: Option<&str>| VoiceChoice { engine: CHATTERBOX.into(), reference_clip: clip.map(PathBuf::from), ..VoiceChoice::default() };
        assert!(r.check(&cb(None)).unwrap_err().contains("choose a reference clip"));
        assert!(r.check(&cb(Some("Emma.wav"))).is_err(), "a relative path is refused");
        assert!(r.check(&cb(Some("C:\\voices\\Emma.txt"))).is_err());
        r.check(&cb(Some("C:\\voices\\Emma.wav"))).unwrap();
        assert!(registry(false).check(&cb(Some("C:\\voices\\Emma.wav"))).unwrap_err().contains("container is not running"));
    }

    #[test]
    fn when_the_chosen_engine_fails_the_system_voice_speaks_and_says_so() {
        let r = registry(false);
        let (spoken, note) = r.speak("hello", &VoiceChoice { engine: CHATTERBOX.into(), reference_clip: Some(PathBuf::from("C:\\v\\Emma.wav")), rate: 1.2, ..VoiceChoice::default() }).unwrap();
        assert!(spoken.ms > 0 && note.unwrap().contains("system voice was used"));
        let (_, none) = r.speak("hello", &VoiceChoice::default()).unwrap();
        assert!(none.is_none());
        assert!(r.speak("  ", &VoiceChoice::default()).is_err());
    }

    #[test]
    fn a_wav_is_read_for_its_length_and_whether_it_is_silence() {
        assert_eq!(wav_info(&wav(true, 500)).unwrap(), (500, true));
        assert_eq!(wav_info(&wav(false, 250)).unwrap(), (250, false));
        assert!(wav_info(b"not a wav at all, nothing here, not even close to a header....").is_err());
    }
}
