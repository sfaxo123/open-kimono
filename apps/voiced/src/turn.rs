//! Smart Turn endpointing judge (CV2): at a live capture's trailing
//! silence, ask the Pipecat Smart Turn v3 sidecar whether the utterance
//! SO FAR is a complete thought or a mid-sentence pause.
//!
//! The judge is an adapter command in the same shape as the STT/TTS
//! `cmd` engines: a shell string run with the capture-so-far as a
//! 16 kHz mono WAV on stdin, answering one line on stdout:
//!
//!   complete <prob>     the speaker is done — end the capture now
//!   incomplete <prob>   mid-thought pause — keep listening
//!
//! FAIL-OPERATIONAL, by construction: any failure — the command is
//! missing, exits nonzero, times out, or answers garbage — is
//! `Unavailable`, and the caller falls back to the CV0 fixed-silence
//! endpointer for the rest of the capture and journals the fallback.
//! Capture never depends on the model; the model only ever advises.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Hard bound on one consultation. A prediction is only useful while
/// the pause is still young; past this the fallback serves better.
pub const JUDGE_TIMEOUT: Duration = Duration::from_millis(3000);

#[derive(Clone, Debug, PartialEq)]
pub enum JudgeOutcome {
    /// The model says the utterance is complete (probability attached).
    Complete(f32),
    /// The model says the speaker is mid-thought (probability of
    /// "complete" attached, i.e. low).
    Incomplete(f32),
    /// No usable verdict: spawn failure, timeout, nonzero exit, or an
    /// unparseable answer. The caller falls back and journals `why`.
    Unavailable(String),
}

#[derive(Clone, Debug)]
pub struct TurnJudge {
    cmd: String,
}

impl TurnJudge {
    pub fn new(cmd: &str) -> TurnJudge {
        TurnJudge { cmd: cmd.to_string() }
    }

    /// Consult the model about one capture-so-far WAV. Never blocks
    /// longer than JUDGE_TIMEOUT; never panics on a broken adapter.
    pub fn judge(&self, wav: &[u8]) -> JudgeOutcome {
        let mut child = match Command::new("/bin/sh")
            .arg("-c")
            .arg(&self.cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return JudgeOutcome::Unavailable(format!("cannot start {}: {e}", self.cmd)),
        };
        // The WAV can exceed the pipe buffer; feed it from a thread so
        // a slow reader cannot deadlock the consultation.
        let wav = wav.to_vec();
        let mut stdin = child.stdin.take().expect("piped stdin");
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&wav);
        });
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Ok(s),
                Ok(None) => {
                    if started.elapsed() > JUDGE_TIMEOUT {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = writer.join();
                        return JudgeOutcome::Unavailable(format!(
                            "no verdict within {} ms",
                            JUDGE_TIMEOUT.as_millis()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => break Err(e),
            }
        };
        let _ = writer.join();
        let status = match status {
            Ok(s) => s,
            Err(e) => return JudgeOutcome::Unavailable(format!("wait failed: {e}")),
        };
        let mut out = String::new();
        if let Some(mut so) = child.stdout.take() {
            use std::io::Read;
            let _ = so.read_to_string(&mut out);
        }
        if !status.success() {
            return JudgeOutcome::Unavailable(format!(
                "adapter exited with {status}: {}",
                out.trim()
            ));
        }
        parse_verdict(out.trim())
    }
}

/// The adapter's stdout contract: `complete <prob>` / `incomplete
/// <prob>`, first line wins. Anything else is Unavailable — a judge
/// that cannot speak the contract is treated as a judge that is down.
fn parse_verdict(line: &str) -> JudgeOutcome {
    let line = line.lines().next().unwrap_or("").trim();
    let mut parts = line.split_whitespace();
    let (Some(word), Some(prob)) = (parts.next(), parts.next()) else {
        return JudgeOutcome::Unavailable(format!("unparseable verdict {line:?}"));
    };
    let Ok(prob) = prob.parse::<f32>() else {
        return JudgeOutcome::Unavailable(format!("unparseable probability in {line:?}"));
    };
    match word {
        "complete" => JudgeOutcome::Complete(prob),
        "incomplete" => JudgeOutcome::Incomplete(prob),
        other => JudgeOutcome::Unavailable(format!("unknown verdict {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_verdicts() {
        assert_eq!(parse_verdict("complete 0.97"), JudgeOutcome::Complete(0.97));
        assert_eq!(parse_verdict("incomplete 0.03\n"), JudgeOutcome::Incomplete(0.03));
    }

    #[test]
    fn garbage_is_unavailable_not_a_verdict() {
        assert!(matches!(parse_verdict(""), JudgeOutcome::Unavailable(_)));
        assert!(matches!(parse_verdict("error boom"), JudgeOutcome::Unavailable(_)));
        assert!(matches!(parse_verdict("complete soon"), JudgeOutcome::Unavailable(_)));
    }

    #[test]
    fn missing_adapter_is_unavailable() {
        let j = TurnJudge::new("/nonexistent/smartturn-judge.sh");
        assert!(matches!(j.judge(b"RIFF"), JudgeOutcome::Unavailable(_)));
    }

    #[test]
    fn shell_stub_verdict_round_trips() {
        let j = TurnJudge::new("cat > /dev/null; printf 'incomplete 0.12\\n'");
        assert_eq!(j.judge(b"RIFF...."), JudgeOutcome::Incomplete(0.12));
    }
}
