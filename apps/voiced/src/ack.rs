//! Instant acknowledgment (CV0 — conversational voice Phase 0).
//!
//! Silence reads as failure: between the transcript landing and the
//! answer audio existing there is a wait (brain, tools, synthesis)
//! that used to be dead air. The ack fills exactly that wait and
//! nothing more: if the answer audio is not ready `ack_delay_ms`
//! after the turn's STT final, ONE pre-rendered clip ("Got it—" /
//! "One sec—") plays. Never more than one per turn; never when the
//! answer came fast.
//!
//! The decision lives in AckGate, a pure state machine over
//! millisecond offsets, so the cap and the fast-answer case are
//! unit-tested with timestamps, not slept through. The watcher thread
//! is only a clock and a pair of hands: it feeds the gate elapsed
//! time, plays the clip the gate names, and reports the outcome for
//! the journal. Playback respects the same quiet rule as the answer
//! itself (tts::play): in quiet mode the ack is decided and
//! journalled, not heard.

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use trace_events::{HandoffStatus, Op6Event, Payload};

use crate::capture::JournalOut;
use crate::config::Config;

/// Longest the watcher keeps watch. A turn that outlives this is
/// dead on some other axis; the ack must not outlive the process.
const MAX_WATCH_MS: u64 = 300_000;
/// Watcher tick. Fine enough against a 600 ms deadline, coarse
/// enough to cost nothing.
const TICK: Duration = Duration::from_millis(10);

/// The pure ack decision: deadline, one-shot cap, clip choice.
///
/// Time is milliseconds since the turn's STT final, supplied by the
/// caller — the gate never reads a clock itself.
#[derive(Clone, Debug, PartialEq)]
pub struct AckGate {
    delay_ms: u64,
    seed: u64,
    fired: bool,
    ready_ms: Option<u64>,
}

impl AckGate {
    pub fn new(delay_ms: u64, seed: u64) -> AckGate {
        AckGate { delay_ms, seed, fired: false, ready_ms: None }
    }

    /// Record that the answer audio exists (first call wins).
    pub fn answer_ready(&mut self, at_ms: u64) {
        if self.ready_ms.is_none() {
            self.ready_ms = Some(at_ms);
        }
    }

    pub fn ready_ms(&self) -> Option<u64> {
        self.ready_ms
    }

    pub fn fired(&self) -> bool {
        self.fired
    }

    /// Ask, at offset `now_ms`, whether the ack should fire now.
    /// Returns the clip index exactly once per gate: at the first
    /// poll at or past the deadline while no answer is ready. The
    /// clip is chosen by seed so consecutive turns do not always
    /// open with the same words.
    pub fn poll(&mut self, now_ms: u64, clip_count: usize) -> Option<usize> {
        if self.fired || clip_count == 0 || self.ready_ms.is_some() {
            return None;
        }
        if now_ms >= self.delay_ms {
            self.fired = true;
            return Some((self.seed % clip_count as u64) as usize);
        }
        None
    }
}

/// How the watch ended — journalled either way, per the CV0 brief.
#[derive(Clone, Debug, PartialEq)]
pub enum AckOutcome {
    /// The deadline passed with no answer ready; the clip played
    /// (or would have, in quiet mode — `played` says which).
    Fired { clip: String, at_ms: u64, played: bool },
    /// The answer audio was ready in time; no ack was needed.
    NotFired { ready_ms: u64 },
}

/// Play one clip through the configured play command, exactly the
/// way tts::play plays the answer. Quiet mode or no play command
/// means decided-but-silent.
fn play_clip(cfg: &Config, clip: &str) -> bool {
    if cfg.quiet {
        return false;
    }
    let Some(cmd) = cfg.play_cmd.as_ref() else { return false };
    let full = format!("{cmd} {clip}");
    match Command::new("/bin/sh").arg("-c").arg(&full).stdout(Stdio::piped()).stderr(Stdio::piped()).output() {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

/// A running ack watch for one turn. Owns the watcher thread;
/// `settle` ends the watch and writes the journal line.
pub struct AckWatch {
    ready_slot: Arc<Mutex<Option<u64>>>,
    stt_final: Instant,
    delay_ms: u64,
    handle: Option<JoinHandle<AckOutcome>>,
}

impl AckWatch {
    /// Start watching, anchored at the turn's STT final. Returns
    /// None when no ack clips are configured — the pre-CV0 configs
    /// behave exactly as they always did.
    pub fn start(cfg: &Config, stt_final: Instant) -> Option<AckWatch> {
        if cfg.ack_clips.is_empty() {
            return None;
        }
        let ready_slot: Arc<Mutex<Option<u64>>> = Arc::new(Mutex::new(None));
        let slot = ready_slot.clone();
        let clips = cfg.ack_clips.clone();
        let delay_ms = cfg.ack_delay_ms;
        let play_cfg = cfg.clone();
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let handle = std::thread::spawn(move || {
            let mut gate = AckGate::new(delay_ms, seed);
            loop {
                let now_ms = stt_final.elapsed().as_millis() as u64;
                if let Some(rms) = *slot.lock().unwrap() {
                    gate.answer_ready(rms);
                    return AckOutcome::NotFired { ready_ms: rms };
                }
                if let Some(idx) = gate.poll(now_ms, clips.len()) {
                    let clip = clips[idx].clone();
                    let played = play_clip(&play_cfg, &clip);
                    return AckOutcome::Fired { clip, at_ms: now_ms, played };
                }
                if now_ms >= MAX_WATCH_MS {
                    return AckOutcome::NotFired { ready_ms: now_ms };
                }
                std::thread::sleep(TICK);
            }
        });
        Some(AckWatch { ready_slot, stt_final, delay_ms, handle: Some(handle) })
    }

    /// Tell the watcher the answer audio exists.
    pub fn mark_ready(&self) {
        let ms = self.stt_final.elapsed().as_millis() as u64;
        let mut slot = self.ready_slot.lock().unwrap();
        if slot.is_none() {
            *slot = Some(ms);
        }
    }

    /// End the watch: mark ready, join the watcher (an ack already
    /// playing finishes first, so it never overlaps the answer), and
    /// journal the outcome — fired or not fired, either way.
    pub fn settle(mut self, j: &mut JournalOut) {
        self.mark_ready();
        let outcome = match self.handle.take() {
            Some(h) => h.join().unwrap_or(AckOutcome::NotFired { ready_ms: 0 }),
            None => AckOutcome::NotFired { ready_ms: 0 },
        };
        let trace = j.new_trace();
        let (reason, status) = match &outcome {
            AckOutcome::Fired { clip, at_ms, played } => (
                format!(
                    "ack fired {at_ms} ms after stt_final (deadline {} ms): {clip} {}",
                    self.delay_ms,
                    if *played { "played" } else { "decided but not played (quiet or no play command)" }
                ),
                HandoffStatus::Completed,
            ),
            AckOutcome::NotFired { ready_ms } => (
                format!(
                    "ack not fired: answer ready {ready_ms} ms after stt_final, inside the {} ms deadline",
                    self.delay_ms
                ),
                HandoffStatus::Completed,
            ),
        };
        let _ = j.emit(trace, None, None, Payload::Op6(Op6Event::Handoff { destination: "playback:ack".into(), reason, status }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_fire_before_deadline() {
        let mut g = AckGate::new(600, 0);
        assert_eq!(g.poll(0, 2), None);
        assert_eq!(g.poll(599, 2), None);
        assert!(!g.fired());
    }

    #[test]
    fn fires_at_deadline_exactly_once() {
        let mut g = AckGate::new(600, 0);
        assert_eq!(g.poll(600, 2), Some(0));
        assert!(g.fired());
        // The cap: later polls never fire again.
        assert_eq!(g.poll(700, 2), None);
        assert_eq!(g.poll(60_000, 2), None);
    }

    #[test]
    fn fast_answer_suppresses_ack() {
        let mut g = AckGate::new(600, 0);
        g.answer_ready(300);
        assert_eq!(g.ready_ms(), Some(300));
        assert_eq!(g.poll(600, 2), None);
        assert_eq!(g.poll(10_000, 2), None);
        assert!(!g.fired());
    }

    #[test]
    fn ready_after_fire_does_not_unfire() {
        let mut g = AckGate::new(600, 1);
        assert_eq!(g.poll(650, 2), Some(1));
        g.answer_ready(700);
        assert_eq!(g.poll(800, 2), None);
        assert!(g.fired());
    }

    #[test]
    fn clip_choice_rotates_with_seed() {
        let mut a = AckGate::new(100, 0);
        let mut b = AckGate::new(100, 1);
        let mut c = AckGate::new(100, 2);
        assert_eq!(a.poll(100, 2), Some(0));
        assert_eq!(b.poll(100, 2), Some(1));
        assert_eq!(c.poll(100, 2), Some(0));
    }

    #[test]
    fn no_clips_never_fires() {
        let mut g = AckGate::new(0, 0);
        assert_eq!(g.poll(0, 0), None);
        assert_eq!(g.poll(999_999, 0), None);
    }
}
