// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Chatterbox, the voice-cloning speech server that runs in a container on this computer. Same rules as every engine here: plain http
//! to this machine only. The container takes a reference clip once (`POST /upload_reference`), then speaks in that voice
//! (`POST /v1/audio/speech` with the clip's file name as the voice).

use crate::screen::multipart;
use crate::{agent, local_base};
use serde_json::{json, Value};
use std::io::Read;
use std::time::{Duration, Instant};
use xiar_voice::{Spoken, VoiceChoice, VoiceEngine, VoiceInfo, CHATTERBOX};

const MAX_AUDIO: u64 = 64 * 1024 * 1024;

pub struct ChatterboxVoice {
    base: String,
    agent: ureq::Agent,
}

impl ChatterboxVoice {
    /// `url` is the container's address on this computer, such as `http://localhost:8055`.
    pub fn new(url: &str) -> Result<Self, String> {
        Ok(Self { base: local_base(url).map_err(|e| e.to_string())?, agent: agent(Duration::from_secs(300)) })
    }

    fn get_json(&self, path: &str) -> Result<Value, String> {
        let mut r = self.agent.get(&format!("{}{path}", self.base)).call().map_err(|e| format!("the Chatterbox container is not answering at {}: {e}", self.base))?;
        if !(200..300).contains(&r.status().as_u16()) {
            return Err(format!("{path}: HTTP {}", r.status().as_u16()));
        }
        let mut t = String::new();
        r.body_mut().as_reader().take(4 * 1024 * 1024).read_to_string(&mut t).map_err(|e| e.to_string())?;
        serde_json::from_str(&t).map_err(|e| format!("{path}: not JSON: {e}"))
    }

    fn reference_files(&self) -> Result<Vec<String>, String> {
        Ok(self.get_json("/get_reference_files")?.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default())
    }

    /// Hands the container a clip once, so it can be named as the voice.
    fn upload(&self, clip: &std::path::Path) -> Result<String, String> {
        let name = clip.file_name().and_then(|n| n.to_str()).ok_or("the clip has no file name")?.to_string();
        let bytes = std::fs::read(clip).map_err(|e| format!("{}: {e}", clip.display()))?;
        if bytes.len() > 25 * 1024 * 1024 {
            return Err("a reference clip is a short recording (under 25 MB)".into());
        }
        let ctype = if name.to_ascii_lowercase().ends_with(".mp3") { "audio/mpeg" } else { "audio/wav" };
        let body = multipart("xiar-boundary-3e7a91c4", &[], "files", &name, ctype, &bytes);
        let mut r = self.agent.post(&format!("{}/upload_reference", self.base)).header("Content-Type", "multipart/form-data; boundary=xiar-boundary-3e7a91c4").send(&body[..]).map_err(|e| format!("could not send the clip: {e}"))?;
        let status = r.status().as_u16();
        let mut t = String::new();
        let _ = r.body_mut().as_reader().take(1024 * 1024).read_to_string(&mut t);
        if !(200..300).contains(&status) {
            return Err(format!("the container refused the clip (HTTP {status}): {}", t.chars().take(200).collect::<String>()));
        }
        Ok(name)
    }
}

/// Scales 16-bit samples in a WAV file in place (the container has no volume control).
fn apply_volume(wav: &mut [u8], volume: f32) {
    if (volume - 1.0).abs() < 0.01 {
        return;
    }
    if let Some(pos) = wav.windows(4).position(|w| w == b"data") {
        for c in wav[pos + 8..].as_chunks_mut::<2>().0 {
            let s = (f32::from(i16::from_le_bytes([c[0], c[1]])) * volume).round().clamp(-32768.0, 32767.0) as i16;
            c.copy_from_slice(&s.to_le_bytes());
        }
    }
}

impl VoiceEngine for ChatterboxVoice {
    fn id(&self) -> &str {
        CHATTERBOX
    }

    fn label(&self) -> &str {
        "Chatterbox (voice cloning)"
    }

    fn availability(&self) -> Result<(), String> {
        self.get_json("/api/model-info").map(|_| ()).map_err(|e| format!("{e}. Start the Chatterbox container, then try again."))
    }

    fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
        let mut v: Vec<VoiceInfo> = self.reference_files()?.into_iter().map(|f| VoiceInfo { engine: CHATTERBOX.into(), name: f.rsplit_once('.').map_or(f.clone(), |(s, _)| s.to_string()), id: f, language: "en".into(), gender: String::new() }).collect();
        if let Ok(p) = self.get_json("/get_predefined_voices") {
            for x in p.as_array().into_iter().flatten() {
                if let Some(file) = x["filename"].as_str() {
                    v.push(VoiceInfo { engine: CHATTERBOX.into(), id: file.into(), name: x["display_name"].as_str().unwrap_or(file).into(), language: "en".into(), gender: String::new() });
                }
            }
        }
        Ok(v)
    }

    fn clones(&self) -> bool {
        true
    }

    fn speak(&self, text: &str, choice: &VoiceChoice) -> Result<Spoken, String> {
        let started = Instant::now();
        let voice = match &choice.reference_clip {
            Some(clip) => {
                let name = clip.file_name().and_then(|n| n.to_str()).ok_or("the clip has no file name")?.to_string();
                if !self.reference_files()?.contains(&name) {
                    self.upload(clip)?;
                }
                name
            }
            None if !choice.voice_id.is_empty() => choice.voice_id.clone(),
            None => return Err("Chatterbox needs a reference clip or one of its voices".into()),
        };
        let body = json!({"model": "chatterbox", "input": text, "voice": voice, "response_format": "wav", "speed": choice.rate});
        let mut r = self.agent.post(&format!("{}/v1/audio/speech", self.base)).header("Content-Type", "application/json").send(body.to_string().as_bytes()).map_err(|e| format!("the Chatterbox container is not answering: {e}"))?;
        let status = r.status().as_u16();
        let mut wav = Vec::new();
        r.body_mut().as_reader().take(MAX_AUDIO).read_to_end(&mut wav).map_err(|e| format!("the audio broke off: {e}"))?;
        if !(200..300).contains(&status) {
            return Err(format!("Chatterbox answered {status}: {}", String::from_utf8_lossy(&wav).chars().take(200).collect::<String>()));
        }
        apply_volume(&mut wav, choice.volume);
        Ok(Spoken { wav, ms: started.elapsed().as_millis() as u64 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_server_on_this_computer_is_accepted() {
        assert!(ChatterboxVoice::new("http://localhost:8055").is_ok());
        assert!(ChatterboxVoice::new("http://127.0.0.1:8055/").is_ok());
        assert!(ChatterboxVoice::new("http://example.com:8055").is_err());
        assert!(ChatterboxVoice::new("https://localhost:8055").is_err());
    }

    #[test]
    fn volume_scales_the_samples_and_leaves_full_volume_alone() {
        let mut wav = b"RIFF....WAVEfmt ................data....".to_vec();
        wav.extend_from_slice(&1000i16.to_le_bytes());
        wav.extend_from_slice(&(-2000i16).to_le_bytes());
        let original = wav.clone();
        apply_volume(&mut wav, 1.0);
        assert_eq!(wav, original);
        apply_volume(&mut wav, 0.5);
        let n = wav.len();
        assert_eq!((i16::from_le_bytes([wav[n - 4], wav[n - 3]]), i16::from_le_bytes([wav[n - 2], wav[n - 1]])), (500, -1000));
    }

    #[test]
    fn a_container_that_is_not_running_is_a_plain_message() {
        let v = ChatterboxVoice::new("http://127.0.0.1:1").unwrap();
        let e = v.availability().unwrap_err();
        assert!(e.contains("not answering") && e.contains("Start the Chatterbox container"), "{e}");
        assert!(v.voices().is_err());
    }
}
