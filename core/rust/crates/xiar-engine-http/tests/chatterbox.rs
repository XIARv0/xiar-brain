// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The real Chatterbox container, with a character's own voice clip as the reference. Needs the container running:
//! `XIAR_CHATTERBOX_URL=http://localhost:8055 XIAR_REFERENCE_CLIP=<Emma.wav> cargo test -p xiar-engine-http --test chatterbox -- --nocapture`
use xiar_engine_http::voice::ChatterboxVoice;
use xiar_voice::{wav_info, VoiceChoice, VoiceEngine, VoiceRegistry, CHATTERBOX};

#[test]
fn the_container_speaks_in_the_voice_of_a_reference_clip() {
    let (Ok(url), Ok(clip)) = (std::env::var("XIAR_CHATTERBOX_URL"), std::env::var("XIAR_REFERENCE_CLIP")) else {
        eprintln!("XIAR_CHATTERBOX_URL / XIAR_REFERENCE_CLIP not set: skipping");
        return;
    };
    let engine = ChatterboxVoice::new(&url).unwrap();
    engine.availability().unwrap();
    let choice = VoiceChoice { engine: CHATTERBOX.into(), reference_clip: Some(clip.into()), ..VoiceChoice::default() };
    let mut reg = VoiceRegistry::new();
    reg.add(std::sync::Arc::new(engine));
    reg.check(&choice).unwrap();
    let t = std::time::Instant::now();
    let (spoken, note) = reg.speak("Remember that your appointment is on Friday at three in the afternoon.", &choice).unwrap();
    assert!(note.is_none(), "{note:?}");
    let (ms, loud) = wav_info(&spoken.wav).unwrap();
    eprintln!("spoke {ms} ms of audio in {:.1} s", t.elapsed().as_secs_f32());
    assert!(loud && ms > 1500, "{ms} ms, loud={loud}");
    // the clip is now one of the container's voices
    let voices = reg.engine(CHATTERBOX).unwrap().voices().unwrap();
    assert!(voices.iter().any(|v| v.id.to_lowercase().ends_with("emma.wav")), "{voices:?}");
    // a quieter ask is quieter
    let (quiet, _) = reg.speak("Remember that your appointment is on Friday at three in the afternoon.", &VoiceChoice { volume: 0.25, ..choice }).unwrap();
    let peak = |w: &[u8]| w[44..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap_or(0);
    assert!(peak(&quiet.wav) < peak(&spoken.wav), "quiet {} loud {}", peak(&quiet.wav), peak(&spoken.wav));
}
