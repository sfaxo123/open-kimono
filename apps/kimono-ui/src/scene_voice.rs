//! Scene: a voice request — mic, live transcript, cleaned text, wake locks,
//! power and service lifecycle. This is the A4 half of the kimono view: the
//! user must always see whether the mic is live and what the STT heard.

use trace_events::{Op6Event, PowerState};

use crate::ev::*;
use crate::journal::Journal;

const T0: u64 = 1_762_103_400_000_000;

pub fn build() -> Journal {
    let mut j = Journal::new(T0);
    j.service("voiced").trace("9a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d");

    j.push(op6(Op6Event::PowerChange {
        component: "mic".into(),
        state: PowerState::On,
        reason: "voiced opened capture for a voice request".into(),
    }));
    j.push(op6(Op6Event::WakeLockAcquired {
        holder: "voiced".into(),
        deadline_us: Some(T0 + 15_000_000),
        max_us: Some(15_000_000),
    }));
    j.push(op6(Op6Event::MicStarted { device: Some("built-in mic (hw:0,6)".into()) }));

    for level in [0.02f32, 0.31, 0.55, 0.47, 0.08] {
        j.tick(60_000);
        j.push(op6(Op6Event::MicFrame { level }));
    }
    for text in ["remind", "remind me to drink", "remind me to drink water"] {
        j.tick(400_000);
        j.push(op6(Op6Event::SttPartial { engine: "moonshine-tiny".into(), text: text.into(), confidence: Some(0.42) }));
    }
    j.tick(350_000);
    j.push(op6(Op6Event::SttFinal {
        engine: "moonshine-tiny".into(),
        text: "remind me to drink water".into(),
        confidence: Some(0.83),
        latency_us: Some(1_180_000),
    }));
    j.push(op6(Op6Event::TextCleaned {
        raw: "um remind me to drink water yeah".into(),
        cleaned: "remind me to drink water".into(),
    }));
    j.push(op6(Op6Event::MicStopped { reason: "end of speech: 800 ms of silence".into() }));
    j.push(op6(Op6Event::PowerChange {
        component: "mic".into(),
        state: PowerState::Off,
        reason: "transcript final; capture closed so the mic is never silently live".into(),
    }));

    // voiced hands the cleaned text to agentd, which starts the service.
    j.set_service("agentd");
    j.push(op6(Op6Event::ServiceStarted {
        service: "water-reminder".into(),
        generation: 2,
        reason: "voice request created it; svcd admitted the IR after simulation".into(),
    }));
    j.push(op6(Op6Event::GrantGranted {
        service: "water-reminder".into(),
        capability: "notify".into(),
        scope: Some("text + quiet tone".into()),
        expires_us: Some(T0 + 86_400_000_000),
    }));
    j.push(op6(Op6Event::GrantGranted {
        service: "water-reminder".into(),
        capability: "wake-lock".into(),
        scope: None,
        expires_us: None,
    }));

    j.set_service("powerd");
    j.tick(2_000_000);
    j.push(op6(Op6Event::PowerChange {
        component: "bluetooth".into(),
        state: PowerState::Off,
        reason: "idle 300 s and no running service declares a bluetooth need".into(),
    }));
    j.push(op6(Op6Event::WakeLockReleased { holder: "voiced".into(), reason: Some("request complete".into()) }));

    j.set_service("agentd");
    j.tick(120_000_000);
    j.push(op6(Op6Event::ServiceStopped {
        service: "water-reminder".into(),
        generation: 2,
        reason: "user said \"stop the water reminder\"".into(),
    }));
    j.push(op6(Op6Event::GrantRevoked {
        service: "water-reminder".into(),
        capability: "notify".into(),
        reason: "service stopped".into(),
    }));
    j.push(op6(Op6Event::GrantRevoked {
        service: "water-reminder".into(),
        capability: "wake-lock".into(),
        reason: "service stopped".into(),
    }));
    j
}
