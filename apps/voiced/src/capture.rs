//! Capture and mic ownership (PLAN A4: "mic ownership by this supervised
//! service, not the browser").
//!
//! A lock file names the live holder; a second voiced refuses instead of
//! sharing the device, because two capture sources on a phone is exactly the
//! kind of thing the user must never have to discover by hearing an echo.
//! Every capture opens and closes in the journal, and closing always carries
//! a reason.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use trace_events::{HandoffStatus, Op6Event, Payload};
use voice_bridge::journal::Journal;
use voice_bridge::wav::{chunk_rms_level, read_pcm, write_pcm, Pcm};

use crate::error::VoiceError;
use crate::turn::{JudgeOutcome, TurnJudge};

pub type JournalOut = Journal<Box<dyn std::io::Write>>;

/// 100 ms of 16 kHz mono s16le — the frame the mic level meter is driven by.
pub const FRAME_BYTES: usize = 3200;

/// A held mic lock. Removing it on drop is what makes "the mic is off" a
/// fact the next process can trust.
pub struct MicGuard {
    lock: PathBuf,
}

impl MicGuard {
    pub fn path(&self) -> &Path {
        &self.lock
    }
}

impl Drop for MicGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock);
    }
}

fn pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// The user's mic switch: the one source of truth for whether voiced may
/// listen at all. It is a one-word file (`on` / `off`) so the kimono view,
/// a shell one-liner, and voiced itself all read the same fact; there is
/// no second copy of this state to drift from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicState {
    On,
    Off,
}

/// Read the mic switch. No file means the mic was never turned off, so
/// it is on. A file that says anything else fails closed: an unreadable
/// switch must never become permission to listen.
pub fn mic_state(state_file: &Path) -> Result<MicState, VoiceError> {
    match std::fs::read_to_string(state_file) {
        Ok(text) => match text.trim().to_ascii_lowercase().as_str() {
            "on" => Ok(MicState::On),
            "off" => Ok(MicState::Off),
            other => Err(VoiceError::Config(format!(
                "mic state file {} must say 'on' or 'off', got {other:?}",
                state_file.display()
            ))),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(MicState::On),
        Err(e) => Err(VoiceError::Io {
            op: format!("read mic state {}", state_file.display()),
            detail: e.to_string(),
        }),
    }
}

/// Take the mic, or name whoever holds it.
pub fn acquire_mic(lock: &Path, holder: &str) -> Result<MicGuard, VoiceError> {
    let busy = |why: &str| VoiceError::MicBusy { holder: why.into(), lock: lock.display().to_string() };
    for attempt in 0..2 {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(lock) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(format!("{} {}\n", std::process::id(), holder).as_bytes())
                    .map_err(|e| VoiceError::Io { op: format!("write lock {}", lock.display()), detail: e.to_string() })?;
                return Ok(MicGuard { lock: lock.to_path_buf() });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let held = std::fs::read_to_string(lock).unwrap_or_default();
                let (pid_field, who) = held.trim().split_once(' ').unwrap_or((held.trim(), ""));
                let pid: u32 = pid_field.parse().unwrap_or(0);
                if pid != 0 && pid_alive(pid) {
                    let who = if who.trim().is_empty() { format!("pid {pid}") } else { who.trim().to_string() };
                    return Err(busy(&who));
                }
                if attempt == 1 {
                    return Err(busy("lock exists and cannot be cleared"));
                }
                std::fs::remove_file(lock)
                    .map_err(|e| VoiceError::Io { op: format!("clear stale lock {}", lock.display()), detail: e.to_string() })?;
            }
            Err(e) => return Err(VoiceError::Io { op: format!("open lock {}", lock.display()), detail: e.to_string() }),
        }
    }
    Err(busy("unreachable"))
}

/// Trailing-silence endpointing for live capture (CV0).
///
/// The phrase-only fired turn used to record the full fixed window
/// (8 s) no matter when the speaker finished: pure dead air on the
/// most common interaction shape. This is the wake-word listener's
/// energy VAD (wakeword.py `Vad`) reduced to its endpoint decision:
/// an adaptive noise floor, a threshold of max(rms_min, floor * 3),
/// speech after 2 consecutive above-threshold frames, and an
/// endpoint after ENDPOINT_SILENCE_FRAMES of trailing silence.
///
/// KNOWN LIMIT (CV0 brief, record-don't-fix): a mid-sentence pause
/// longer than the trailing-silence window is indistinguishable
/// from "done" and ends the capture early. Judging *incomplete*
/// speech is CV2's Smart Turn job; the unit tests pin this cut so
/// it is a documented behaviour, not a surprise.
pub struct Endpointer {
    floor: f32,
    above: u32,
    in_speech: bool,
    below: u32,
}

/// 7 frames x 100 ms = ~0.7 s of trailing silence ends the capture.
pub const ENDPOINT_SILENCE_FRAMES: u32 = 7;
/// Speech must clear this RMS level: the listener's 300 (raw s16)
/// floor, normalised.
pub const SPEECH_RMS_MIN: f32 = 300.0 / 32768.0;
/// Frames of speech onset before the capture counts as started
/// (the listener's `above >= 2` rule).
pub const SPEECH_ONSET_FRAMES: u32 = 2;

impl Endpointer {
    pub fn new() -> Endpointer {
        Endpointer { floor: 150.0 / 32768.0, above: 0, in_speech: false, below: 0 }
    }

    fn threshold(&self) -> f32 {
        (self.floor * 3.0).max(SPEECH_RMS_MIN)
    }

    pub fn in_speech(&self) -> bool {
        self.in_speech
    }

    /// Feed one 100 ms frame's RMS level; returns true exactly when
    /// the capture should end (trailing silence after speech).
    pub fn feed(&mut self, level: f32) -> bool {
        let thr = self.threshold();
        if !self.in_speech {
            if level <= thr {
                self.floor = self.floor * 0.95 + level * 0.05;
            }
            if level > thr {
                self.above += 1;
                if self.above >= SPEECH_ONSET_FRAMES {
                    self.in_speech = true;
                    self.below = 0;
                }
            } else {
                self.above = 0;
            }
            return false;
        }
        if level > thr {
            self.below = 0;
        } else {
            self.below += 1;
        }
        self.below >= ENDPOINT_SILENCE_FRAMES
    }
}

/// The CV0 endpoint reason, word for word: the fallback path and the
/// no-judge path both end captures with exactly this string.
pub const CV0_STOP_REASON: &str = "vad endpoint: 0.7 s trailing silence after speech (cv0 endpointing)";

/// First Smart Turn consultation at 0.2 s of trailing silence: with
/// one warm prediction (~0.3 s measured on-device) a confident
/// complete ends the turn ~0.5 s after the last word, inside the CV2
/// brief's 0.6 s budget; and the earlier the first verdict, the
/// fresher the continuation prosody it judges (see the anchor note
/// at SMART_TURN_CONFIDENT).
pub const SMART_TURN_FIRST_CONSULT_FRAMES: u32 = 2;
/// While the model keeps saying "incomplete", re-consult every 0.5 s
/// of continued silence.
pub const SMART_TURN_RECONSULT_FRAMES: u32 = 5;
/// Never wait past 2.5 s of trailing silence, whatever the model says:
/// a speaker who never resumes still ends the capture.
pub const SMART_TURN_MAX_TRAILING_FRAMES: u32 = 25;
/// A "complete" verdict at or above this probability is CONFIDENT:
/// real completions score 0.95+ on the deployed model.
/// How verdicts may end a capture is anchored on the FIRST verdict of
/// each pause (see JudgedEndpoint): ONLY a confident first verdict
/// may end a pause. Measured on the CV0 pause fixture, the model's
/// probability climbs with silence length alone (0.32 -> 0.86 -> 0.99
/// across one 1.5 s pause after a complete-sounding clause), and its
/// early readings are jagged (0.64 at 0.2 s of silence vs 0.32 at
/// 0.3 s on the same audio): any rule that lets a later verdict of
/// the same pause end the capture - including a "confirming" second
/// complete - eventually cuts a pause the first verdict would have
/// ridden out. A first verdict that is incomplete OR middling
/// therefore LOCKS the pause: it ends only when speech resumes or
/// the 2.5 s trailing cap is reached. The cost asymmetry is
/// deliberate: a false end mid-thought is the CV0 defect itself; a
/// false wait (a finished request whose first verdict was middling
/// rides to the cap) is bounded dead air, journalled either way.
pub const SMART_TURN_CONFIDENT: f32 = 0.9;

/// Where one pause stands, anchored on its first verdict.
#[derive(Clone, Copy, PartialEq)]
enum Episode {
    Undecided,
    Locked,
}

/// CV2 judged endpointing: the Smart Turn model advises the CV0
/// endpointer. State per capture; a failed consultation switches to
/// `fallback`, after which the plain CV0 rule decides — and the
/// fallback is journalled loudly, never silent.
struct JudgedEndpoint {
    next_consult: u32,
    fallback: bool,
    last_prob: Option<f32>,
    /// The current pause's anchored stance (first verdict decides:
    /// a confident complete ends it, anything else locks it).
    episode: Episode,
}

impl JudgedEndpoint {
    fn new() -> JudgedEndpoint {
        JudgedEndpoint {
            next_consult: SMART_TURN_FIRST_CONSULT_FRAMES,
            fallback: false,
            last_prob: None,
            episode: Episode::Undecided,
        }
    }

    /// One fed frame's endpoint decision. `below` is the endpointer's
    /// trailing-silence count, `ended` its CV0 verdict, `samples` the
    /// capture so far (raw s16le). Some(reason) ends the capture.
    fn on_frame(
        &mut self,
        judge: &TurnJudge,
        below: u32,
        ended: bool,
        samples: &[u8],
        j: &mut JournalOut,
        trace: trace_events::TraceId,
    ) -> Option<String> {
        if below == 0 {
            // Speech (re)started: the next pause earns a fresh first
            // consultation at 0.3 s, not whatever the last pause's
            // schedule had reached, and the last pause's anchored
            // stance is moot.
            self.next_consult = SMART_TURN_FIRST_CONSULT_FRAMES;
            self.episode = Episode::Undecided;
        }
        if self.fallback {
            return ended.then(|| CV0_STOP_REASON.to_string());
        }
        if below >= self.next_consult && below < SMART_TURN_MAX_TRAILING_FRAMES {
            match consult(judge, samples) {
                JudgeOutcome::Complete(p) => {
                    self.last_prob = Some(p);
                    match self.episode {
                        Episode::Undecided => {
                            if p >= SMART_TURN_CONFIDENT {
                                return Some(format!(
                                    "smart turn complete (p={p:.2}) at {:.1} s trailing silence (cv2 smart turn)",
                                    below as f32 / 10.0
                                ));
                            }
                            self.episode = Episode::Locked;
                            self.next_consult = below + SMART_TURN_RECONSULT_FRAMES;
                            let _ = emit(
                                j,
                                trace,
                                Payload::Op6(Op6Event::Handoff {
                                    destination: "smart-turn".into(),
                                    reason: format!(
                                        "complete (p={p:.2}) on this pause's first verdict, below the confident bar {SMART_TURN_CONFIDENT}: pause locked, still listening"
                                    ),
                                    status: HandoffStatus::Completed,
                                }),
                            );
                        }
                        Episode::Locked => {
                            // A locked pause is not ended by any
                            // later verdict of the same pause: they
                            // measure silence length, not intent.
                            self.next_consult = below + SMART_TURN_RECONSULT_FRAMES;
                            let _ = emit(
                                j,
                                trace,
                                Payload::Op6(Op6Event::Handoff {
                                    destination: "smart-turn".into(),
                                    reason: format!(
                                        "complete (p={p:.2}) in a locked pause (first verdict did not end it): holding (first-verdict anchor), still listening"
                                    ),
                                    status: HandoffStatus::Completed,
                                }),
                            );
                        }
                    }
                }
                JudgeOutcome::Incomplete(p) => {
                    if self.episode == Episode::Undecided {
                        self.episode = Episode::Locked;
                    }
                    self.last_prob = Some(p);
                    self.next_consult = below + SMART_TURN_RECONSULT_FRAMES;
                    let _ = emit(
                        j,
                        trace,
                        Payload::Op6(Op6Event::Handoff {
                            destination: "smart-turn".into(),
                            reason: format!(
                                "incomplete (p={p:.2}) at {:.1} s trailing silence: still listening",
                                below as f32 / 10.0
                            ),
                            status: HandoffStatus::Completed,
                        }),
                    );
                }
                JudgeOutcome::Unavailable(why) => {
                    self.fallback = true;
                    let _ = emit(
                        j,
                        trace,
                        Payload::Op6(Op6Event::Handoff {
                            destination: "smart-turn".into(),
                            reason: format!("smart turn unavailable ({why}); cv0 endpointing fallback"),
                            status: HandoffStatus::Failed,
                        }),
                    );
                    return ended.then(|| CV0_STOP_REASON.to_string());
                }
            }
        }
        if below >= SMART_TURN_MAX_TRAILING_FRAMES {
            return Some(format!(
                "smart turn: no confident end by 2.5 s max trailing silence (last p={:.2}); ending (cv2 smart turn)",
                self.last_prob.unwrap_or(0.0)
            ));
        }
        // Between the CV0 endpoint (0.7 s) and the next consultation the
        // model has said "not done": keep listening.
        None
    }
}

/// Stage the capture-so-far as a WAV and ask the judge about it.
fn consult(judge: &TurnJudge, samples: &[u8]) -> JudgeOutcome {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("voiced-smartturn-{}-{nonce}.wav", std::process::id()));
    if let Err(e) = write_pcm(&tmp, 16_000, 2, 1, samples) {
        return JudgeOutcome::Unavailable(format!("cannot stage audio: {e}"));
    }
    let wav = std::fs::read(&tmp).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    judge.judge(&wav)
}

fn emit(j: &mut JournalOut, trace: trace_events::TraceId, payload: Payload) -> Result<(), VoiceError> {
    j.emit(trace, None, None, payload).map_err(|e| VoiceError::Io { op: "journal".into(), detail: e.to_string() })
}

/// Capture from a recorded WAV — the input the phone safety rules require for
/// automated tests (no live mic).
pub fn capture_wav(path: &str, j: &mut JournalOut) -> Result<Pcm, VoiceError> {
    let trace = j.new_trace();
    emit(j, trace, Payload::Op6(Op6Event::MicStarted { device: Some(format!("file:{path}")) }))?;
    let pcm = match read_pcm(path) {
        Ok(p) => p,
        Err(e) => {
            let _ = emit(j, trace, Payload::Op6(Op6Event::MicStopped { reason: format!("capture failed: {e}") }));
            return Err(VoiceError::Capture(e));
        }
    };
    for chunk in pcm.samples.chunks(FRAME_BYTES) {
        let level = chunk_rms_level(chunk);
        emit(j, trace, Payload::Op6(Op6Event::MicFrame { level }))?;
    }
    emit(j, trace, Payload::Op6(Op6Event::MicStopped { reason: "file input consumed to the end".into() }))?;
    Ok(pcm)
}

/// Capture from the live mic through a command that writes raw s16le mono
/// 16 kHz to stdout. Only reached when the operator asks for it explicitly.
pub fn capture_mic(cmd: &str, max_seconds: u64, judge: Option<&TurnJudge>, j: &mut JournalOut) -> Result<Pcm, VoiceError> {
    let trace = j.new_trace();
    emit(j, trace, Payload::Op6(Op6Event::MicStarted { device: Some(cmd.to_string()) }))?;
    let cap = (max_seconds as usize) * 32_000;
    let mut child = match Command::new("/bin/sh").arg("-c").arg(cmd).stdout(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = emit(j, trace, Payload::Op6(Op6Event::MicStopped { reason: format!("capture command failed to start: {e}") }));
            return Err(VoiceError::Capture(format!("{cmd}: {e}")));
        }
    };
    let mut buf = Vec::new();
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut frame = [0u8; FRAME_BYTES];
    let stopped: String;
    // CV0 endpointing: whole 100 ms frames are fed to the endpointer
    // as they assemble in the buffer; reads from a pipe may be
    // partial, so frame boundaries come from the byte count, not
    // from read() calls.
    let mut endpointer = Endpointer::new();
    let mut judged = judge.map(|_| JudgedEndpoint::new());
    let mut fed = 0usize;
    loop {
        match stdout.read(&mut frame) {
            Ok(0) => {
                stopped = "capture command closed the stream".to_string();
                break;
            }
            Ok(n) => {
                buf.extend_from_slice(&frame[..n]);
                let level = chunk_rms_level(&frame[..n]);
                emit(j, trace, Payload::Op6(Op6Event::MicFrame { level }))?;
                let mut stop: Option<String> = None;
                while buf.len() - fed >= FRAME_BYTES {
                    let lvl = chunk_rms_level(&buf[fed..fed + FRAME_BYTES]);
                    fed += FRAME_BYTES;
                    let ended = endpointer.feed(lvl);
                    if endpointer.in_speech() {
                        match (judge, judged.as_mut()) {
                            (Some(jd), Some(je)) => {
                                if let Some(reason) = je.on_frame(jd, endpointer.below, ended, &buf[..fed], j, trace) {
                                    stop = Some(reason);
                                    break;
                                }
                            }
                            _ => {
                                if ended {
                                    stop = Some(CV0_STOP_REASON.to_string());
                                    break;
                                }
                            }
                        }
                    }
                }
                if let Some(reason) = stop {
                    stopped = reason;
                    break;
                }
                if buf.len() >= cap {
                    stopped = format!("capture limit of {max_seconds} s reached");
                    break;
                }
            }
            Err(e) => {
                stopped = format!("capture read failed: {e}");
                break;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    emit(j, trace, Payload::Op6(Op6Event::MicStopped { reason: stopped }))?;
    if buf.is_empty() {
        return Err(VoiceError::Capture("capture produced no bytes".into()));
    }
    Ok(Pcm { rate: 16_000, channels: 1, width: 2, samples: buf })
}

#[cfg(test)]
mod endpointer_tests {
    use super::*;

    const SPEECH: f32 = 0.08;
    const QUIET: f32 = 0.001;

    fn feed_n(ep: &mut Endpointer, level: f32, n: u32) -> bool {
        let mut ended = false;
        for _ in 0..n {
            ended = ep.feed(level) || ended;
        }
        ended
    }

    #[test]
    fn endpoints_after_point_seven_seconds_of_trailing_silence() {
        let mut ep = Endpointer::new();
        assert!(!feed_n(&mut ep, SPEECH, 10)); // 1.0 s of speech
        assert!(ep.in_speech());
        assert!(!feed_n(&mut ep, QUIET, 6)); // 0.6 s: not yet
        assert!(ep.feed(QUIET)); // 0.7 s: endpoint
    }

    #[test]
    fn short_pause_does_not_cut() {
        let mut ep = Endpointer::new();
        assert!(!feed_n(&mut ep, SPEECH, 10));
        assert!(!feed_n(&mut ep, QUIET, 4)); // 0.4 s pause
        assert!(!feed_n(&mut ep, SPEECH, 10)); // speaking again
        assert!(!feed_n(&mut ep, QUIET, 6));
        assert!(ep.feed(QUIET));
    }

    #[test]
    fn silence_alone_never_endpoints() {
        let mut ep = Endpointer::new();
        // Two minutes of quiet: the window ceiling, not the
        // endpointer, ends a speechless capture.
        assert!(!feed_n(&mut ep, QUIET, 1200));
        assert!(!ep.in_speech());
    }

    #[test]
    fn deliberate_long_pause_is_cut_known_cv2_limit() {
        // The CV0 brief's known limit, pinned: a 1.5 s mid-sentence
        // pause ends the capture at the 0.7 s mark. CV2's Smart
        // Turn is the fix when a judge is configured (the judged
        // capture tests in tests/pipeline.rs prove it); with no
        // judge — and after any judge failure — THIS behaviour is
        // the fallback, and it stays pinned so the fallback can
        // never silently drift.
        let mut ep = Endpointer::new();
        assert!(!feed_n(&mut ep, SPEECH, 10)); // "open maps and give me directions to..."
        assert!(feed_n(&mut ep, QUIET, 15)); // 1.5 s pause -> cut
    }
}
