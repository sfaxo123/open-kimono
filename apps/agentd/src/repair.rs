//! CV2 ASR-confidence repair strata: what agentd does when the words
//! it was handed may not be the words the user said.
//!
//! BRAINFIX governs PROVENANCE — whether a value the agent inferred
//! may act without the owner's read-back. This module governs a
//! different axis: TRANSCRIPTION confidence — whether a value the
//! user stated was heard correctly at all. The axes compose; neither
//! replaces the other (see the CV2 addendum in
//! docs/workbench/user-simulations/key-tests/BRAINFIX-INSTRUCTION.md).
//!
//! THE SIGNAL, HONESTLY: the only confidence that reaches agentd
//! today is the one voiced puts on the wire (`asr_confidence`), and
//! voiced's value is an ACOUSTIC-LEVEL PROXY — the whole utterance's
//! RMS level mapped to 0.8 (clearly audible) or 0.4 (quiet). It is
//! NOT a decoder confidence: the Moonshine driver contract exposes
//! no per-word or per-utterance score, and no finer signal exists on
//! the live path. The strata are written against whatever value the
//! wire carries, so a finer signal later (a decoder score, a
//! per-span score) needs no redesign — only the thresholds move.
//! A request with NO signal behaves exactly as agentd did before
//! CV2: the strata never invent doubt the caller did not report.
//!
//! The strata:
//! - HIGH — act exactly as today.
//! - MEDIUM — proceed, but the brain is told (as background data,
//!   never as system prompt) to naturally restate the heard span in
//!   its answer while carrying out the request.
//! - LOW — do not run the brain and do not touch a tool. Ask exactly
//!   ONE targeted question about the uncertain span, keep the
//!   session's slots, and resume the original request from the
//!   spoken answer. The answer is accepted as spoken — a repair
//!   never asks twice.

/// Which stratum a turn falls in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stratum {
    High,
    Medium,
    Low,
}

impl Stratum {
    /// The journal/config vocabulary for the stratum.
    pub fn name(self) -> &'static str {
        match self {
            Stratum::High => "high",
            Stratum::Medium => "medium",
            Stratum::Low => "low",
        }
    }
}

/// What kind of span the extractor found uncertain-worthy. The kind
/// shapes the targeted question and how the answer composes back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanKind {
    /// The destination of a directions/navigation request.
    Destination,
    /// The app of an open/launch request.
    App,
    /// No known shape: the whole request is the span.
    Generic,
}

/// A repair waiting on its answer, stored in the session beside the
/// pending list. `original` is the request as transcribed; `span` is
/// the part the question asked about, as heard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingRepair {
    pub original: String,
    pub span: String,
    pub kind: SpanKind,
}

/// Classify one turn. `high`/`low` are the configured thresholds:
/// at or above `high` is HIGH, below `low` is LOW, between is
/// MEDIUM. No signal classifies HIGH with a reason that says so —
/// fail-operational means yesterday's behavior, never a question
/// invented from missing data. The reason is journaled verbatim.
pub fn classify(confidence: Option<f32>, high: f32, low: f32) -> (Stratum, String) {
    match confidence {
        None => (Stratum::High, "no asr_confidence on the wire: no signal, behaving as before CV2".to_string()),
        Some(c) if c >= high => (Stratum::High, format!("confidence {c:.2} >= high threshold {high:.2}: act as today")),
        Some(c) if c < low => (Stratum::Low, format!("confidence {c:.2} < low threshold {low:.2}: ask one targeted question")),
        Some(c) => (
            Stratum::Medium,
            format!("confidence {c:.2} in the medium band [{low:.2}, {high:.2}): proceed, restating the heard span"),
        ),
    }
}

/// Find the span a repair question should target in `text`. Small
/// and deterministic on purpose: this runs where the brain may not,
/// so it must never itself need a model.
pub fn extract_span(text: &str) -> (SpanKind, String) {
    let lower = text.to_lowercase();
    let clean = |s: &str| s.trim().trim_end_matches(['.', '!', '?', ',']).trim().to_string();
    for marker in ["directions to ", "direction to ", "navigate to ", "take me to ", "route to "] {
        if let Some(idx) = lower.find(marker) {
            let span = clean(&text[idx + marker.len()..]);
            if !span.is_empty() {
                return (SpanKind::Destination, span);
            }
        }
    }
    if lower.contains("direction") || lower.contains("maps") {
        if let Some(idx) = lower.rfind(" to ") {
            let span = clean(&text[idx + 4..]);
            if !span.is_empty() {
                return (SpanKind::Destination, span);
            }
        }
    }
    for verb in ["open ", "launch ", "start "] {
        if lower.starts_with(verb) {
            let rest = text[verb.len()..].split_whitespace().take(2).collect::<Vec<_>>().join(" ");
            let span = clean(&rest);
            if !span.is_empty() {
                return (SpanKind::App, span);
            }
        }
    }
    (SpanKind::Generic, clean(text))
}

/// The ONE targeted question a LOW turn asks. It names the span as
/// heard and asks only about that span — never a re-ask of the
/// whole request, never a second question.
pub fn question_for(kind: SpanKind, span: &str) -> String {
    match kind {
        SpanKind::Destination => format!(
            "I heard the destination as \"{span}\", but I'm not confident I got it right. Where did you want directions to?"
        ),
        SpanKind::App => format!(
            "I heard \"{span}\", but I'm not confident I got the app right. Which app should I open?"
        ),
        SpanKind::Generic => format!(
            "I didn't catch that clearly - I heard \"{span}\". Could you say again what you'd like?"
        ),
    }
}

/// The background note a MEDIUM turn adds for the brain: proceed with
/// the request, and restate the heard span naturally while doing
/// so. Data, not instruction hierarchy: it rides as a user-role
/// note like the session entity summary, and it explicitly does not
/// substitute for any confirmation BRAINFIX or the broker requires.
pub fn medium_note(span: &str) -> String {
    format!(
        "Transcription note (background, not a new request): this request arrived with medium \
         transcription confidence; the uncertain part as heard is \"{span}\". Carry out the request \
         as normal, and in your spoken answer naturally restate that part as heard (for example, \
         \"...to {span}...\") so the user can catch a mishearing. This note changes nothing about \
         confirmation: any confirmation the request itself requires still applies exactly as usual."
    )
}

/// Compose the request a repair answer resumes: the original text
/// with the heard span replaced by what the user actually said. A
/// confirming answer ("yes", "that's right") keeps the original
/// text as spoken — the user confirmed the span, so nothing was
/// misheard after all. If the span cannot be found (it should not
/// happen; the extractor produced it from this text), the answer
/// stands alone as the request rather than being dropped.
///
/// Provenance honesty: the composed text contains only what the
/// user said across the two turns — the original transcription and
/// the spoken answer. A repair never promotes an agent-inferred
/// value into the user's words; BRAINFIX adjudicates anything the
/// agent inferred exactly as it did before CV2.
pub fn compose_resume(pending: &PendingRepair, answer: &str) -> String {
    let a = answer.trim().trim_end_matches(['.', '!', '?']).trim();
    let low = a.to_lowercase();
    if matches!(
        low.as_str(),
        "yes" | "yeah" | "yep" | "correct" | "right" | "that's right" | "that is right" | "yes it is" | "it is"
    ) {
        return pending.original.clone();
    }
    if !pending.span.is_empty() {
        let orig_lower = pending.original.to_lowercase();
        let span_lower = pending.span.to_lowercase();
        if let Some(idx) = orig_lower.find(&span_lower) {
            let mut out = String::with_capacity(pending.original.len() + a.len());
            out.push_str(&pending.original[..idx]);
            out.push_str(a);
            out.push_str(&pending.original[idx + pending.span.len()..]);
            return out;
        }
    }
    a.to_string()
}
