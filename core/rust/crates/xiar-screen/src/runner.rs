// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The loop that drives the screen and voice monitor: every round it asks the monitor for one capture (the monitor decides
//! whether anything may be looked at at all: off, paused, too soon, sensitive window, duplicate) and one listening step, and
//! hands what comes back to an [`ObservationSink`] (the memory pipeline, or a holding queue until it exists).
//!
//! Rounds are short (a second by default) so that switching the monitor off or pausing it takes effect, and the microphone is
//! released, within a round; the monitor itself spaces the actual captures by its own interval.

use crate::pipeline::{Capture, Microphone, Monitor, Observation, Transcriber};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Where finished observations go.
pub trait ObservationSink: Send + Sync {
    /// Take one observation. `Ok(true)` when it tipped a batch into a memory update.
    fn deliver(&self, o: &Observation) -> Result<bool, String>;
}

/// What one round did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Round {
    /// observations handed to the sink this round
    pub kept: usize,
    /// whether one of them tipped a batch into a memory update
    pub updated_memory: bool,
    /// a failure of the sink (the monitor's own skips are not failures)
    pub error: Option<String>,
}

pub type RoundHook = Arc<dyn Fn(&Round) + Send + Sync>;

pub struct ScreenRunner {
    monitor: Arc<Monitor>,
    capture: Arc<dyn Capture>,
    voice: Option<(Arc<dyn Microphone>, Arc<dyn Transcriber>)>,
    sink: Arc<dyn ObservationSink>,
    on_round: Option<RoundHook>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl ScreenRunner {
    pub fn new(monitor: Arc<Monitor>, capture: Arc<dyn Capture>, voice: Option<(Arc<dyn Microphone>, Arc<dyn Transcriber>)>, sink: Arc<dyn ObservationSink>) -> Self {
        Self { monitor, capture, voice, sink, on_round: None }
    }

    /// Called after every round that kept something or failed (for status displays).
    pub fn with_hook(mut self, hook: RoundHook) -> Self {
        self.on_round = Some(hook);
        self
    }

    fn hand_over(&self, o: &Observation, round: &mut Round) {
        match self.sink.deliver(o) {
            Ok(tipped) => {
                round.kept += 1;
                round.updated_memory |= tipped;
            }
            Err(e) => round.error = Some(e),
        }
    }

    fn report(&self, round: &Round) {
        if round.kept > 0 || round.error.is_some() {
            if let Some(h) = &self.on_round {
                h(round);
            }
        }
    }

    /// One capture step and one listening step at `now`.
    pub fn round(&self, now: u64) -> Round {
        let mut round = Round::default();
        if let Ok(o) = self.monitor.tick(&*self.capture, now) {
            self.hand_over(&o, &mut round);
        }
        if let Some((mic, stt)) = &self.voice {
            if let Some(o) = self.monitor.listen(&**mic, &**stt, now) {
                self.hand_over(&o, &mut round);
            }
        }
        self.report(&round);
        round
    }

    /// End of a session: whatever speech is still waiting is delivered, and the microphone is released.
    pub fn finish(&self) -> Round {
        let mut round = Round::default();
        if let Some(o) = self.monitor.flush_voice() {
            self.hand_over(&o, &mut round);
        }
        if let Some((mic, _)) = &self.voice {
            mic.stop();
        }
        self.report(&round);
        round
    }

    /// Run rounds on a thread every `poll` until the handle is stopped or dropped.
    pub fn spawn(self: Arc<Self>, poll: Duration) -> RunnerHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                self.round(now_ms());
                // sleep in small steps so a stop is noticed quickly
                let mut waited = Duration::ZERO;
                while waited < poll && !flag.load(Ordering::SeqCst) {
                    let step = Duration::from_millis(50).min(poll - waited);
                    std::thread::sleep(step);
                    waited += step;
                }
            }
            self.finish();
        });
        RunnerHandle { stop, handle: Some(handle) }
    }
}

pub struct RunnerHandle {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl RunnerHandle {
    /// Stop and wait: when this returns the loop has ended and the microphone has been released.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for RunnerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}
