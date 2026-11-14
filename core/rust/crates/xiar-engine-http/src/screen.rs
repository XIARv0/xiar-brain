// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The screen monitor's vision and speech steps against a server on THIS machine (Ollama, llama.cpp's `llama-server`, vLLM, a
//! Whisper server in a container...), under the same local-only rules as the chat engine (plain http to loopback, no redirects).
//!
//! * [`HttpDescriber`]: `POST /v1/chat/completions` with the screenshot as a PNG data URL in an `image_url` part, the format Ollama,
//!   llama.cpp and vLLM accept for vision models.
//! * [`HttpTranscriber`]: `POST /v1/audio/transcriptions` as `multipart/form-data` (`file` = a 16-bit mono WAV, `model`), the OpenAI
//!   shape that faster-whisper servers and others speak; the reply's `text` is the transcript.
//!
//! The image and the audio exist only in memory and in the one request.

use crate::{agent, local_base, post_bytes, HttpEngineConfig};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use xiar_llm::LlmError;
use xiar_screen::pipeline::{Clip, Describer, Frame, Transcriber};

/// An RGBA image as PNG bytes.
pub fn png_bytes(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || rgba.len() != (width as usize) * (height as usize) * 4 {
        return Err("the image data does not match its width and height".into());
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// 16-bit mono PCM as a WAV file.
pub fn wav_bytes(sample_rate: u32, pcm16: &[i16]) -> Vec<u8> {
    let data_len = (pcm16.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm16.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm16 {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// `multipart/form-data` with text fields and one file.
pub fn multipart(boundary: &str, fields: &[(&str, &str)], file_field: &str, file_name: &str, file_type: &str, file: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(file.len() + 512);
    for (k, v) in fields {
        b.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    b.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"{file_name}\"\r\nContent-Type: {file_type}\r\n\r\n").as_bytes());
    b.extend_from_slice(file);
    b.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    b
}

pub struct HttpDescriber {
    cfg: HttpEngineConfig,
    base: String,
    agent: ureq::Agent,
    prompt: String,
}

impl HttpDescriber {
    pub fn new(cfg: HttpEngineConfig, prompt: &str) -> Result<Self, LlmError> {
        let base = local_base(&cfg.base_url)?;
        let agent = agent(cfg.timeout);
        Ok(Self { cfg, base, agent, prompt: prompt.to_string() })
    }
}

impl HttpDescriber {
    /// Describes a picture that is already encoded (`image/jpeg` or `image/png`), without decoding it.
    pub fn describe_encoded(&self, mime: &str, bytes: &[u8]) -> Result<String, String> {
        let body = json!({
            "model": self.cfg.model,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": self.prompt},
                {"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{}", B64.encode(bytes))}},
            ]}],
            "temperature": 0.2,
            "max_tokens": 400,
        });
        let v = post_bytes(&self.agent, &format!("{}/v1/chat/completions", self.base), self.cfg.api_key.as_deref(), "application/json", body.to_string().into_bytes()).map_err(|e| e.to_string())?;
        v["choices"][0]["message"]["content"].as_str().map(|s| s.trim().to_string()).ok_or_else(|| "the vision engine's reply has no text".to_string())
    }
}

impl Describer for HttpDescriber {
    fn describe(&self, frame: &Frame) -> Result<String, String> {
        let png = png_bytes(frame.width, frame.height, &frame.rgba)?;
        let body = json!({
            "model": self.cfg.model,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": self.prompt},
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", B64.encode(png))}},
            ]}],
            "temperature": 0.2,
            "max_tokens": 400,
        });
        let v = post_bytes(&self.agent, &format!("{}/v1/chat/completions", self.base), self.cfg.api_key.as_deref(), "application/json", body.to_string().into_bytes()).map_err(|e| e.to_string())?;
        v["choices"][0]["message"]["content"].as_str().map(|s| s.trim().to_string()).ok_or_else(|| "the vision engine's reply has no text".to_string())
    }
}

pub struct HttpTranscriber {
    cfg: HttpEngineConfig,
    base: String,
    agent: ureq::Agent,
}

impl HttpTranscriber {
    pub fn new(cfg: HttpEngineConfig) -> Result<Self, LlmError> {
        let base = local_base(&cfg.base_url)?;
        let agent = agent(cfg.timeout);
        Ok(Self { cfg, base, agent })
    }
}

impl Transcriber for HttpTranscriber {
    fn transcribe(&self, clip: &Clip) -> Result<String, String> {
        let wav = wav_bytes(clip.sample_rate, &clip.pcm16);
        let boundary = "xiar-boundary-7d1c9f3a52b8";
        let body = multipart(boundary, &[("model", &self.cfg.model), ("response_format", "json")], "file", "speech.wav", "audio/wav", &wav);
        let v: Value = post_bytes(&self.agent, &format!("{}/v1/audio/transcriptions", self.base), self.cfg.api_key.as_deref(), &format!("multipart/form-data; boundary={boundary}"), body).map_err(|e| e.to_string())?;
        v["text"].as_str().map(|s| s.trim().to_string()).ok_or_else(|| "the speech engine's reply has no text".to_string())
    }
}
