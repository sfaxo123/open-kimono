//! voiced — A4: capture → STT → agentd request → TTS of the answer, with
//! every step written as a `trace-events` record.
//!
//! The pipeline is a library so the host tests can drive it in-process with
//! WAV fixtures and a fake agentd; `src/main.rs` is the CLI around it.

pub mod ack;
pub mod agentd;
pub mod capture;
pub mod config;
pub mod error;
pub mod stt;
pub mod stream;
pub mod tts;
pub mod turn;

use std::path::Path;
use std::time::{Duration, Instant};

use trace_events::{AgUiEvent, Op6Event, Payload, PowerState};

use crate::capture::JournalOut;
use crate::config::Config;
use crate::error::VoiceError;

/// Where the audio comes from. `Wav` is the only input an automated phone run
/// may use; `Mic` requires the operator to ask for it by name.
#[derive(Clone, Debug)]
pub enum Input {
    Wav { path: String },
    Mic { cmd: String, seconds: u64 },
}

#[derive(Clone, Debug)]
pub struct Request {
    pub input: Input,
    /// Where the spoken answer is written.
    pub out_wav: String,
    /// How long to wait for the agent run to finish.
    pub answer_timeout: Duration,
    /// The CV1 conversation session this turn belongs to, when the
    /// caller (the wakeword listener) is running one. `None` is a
    /// standalone turn: the request goes out exactly as before.
    pub session: Option<String>,
    /// True when the listener accepted this turn without the wake word
    /// inside a follow-up window (CV1). Journaled by agentd as such.
    pub follow_up: bool,
}

/// What the run produced — the tests assert on this, not on stdout.
#[derive(Debug)]
pub struct Outcome {
    pub heard: String,
    pub cleaned: String,
    pub answer: String,
    pub out_wav: String,
}

pub const SERVICE_NAME: &str = "voiced";

fn emit(j: &mut JournalOut, payload: Payload) {
    let trace = j.new_trace();
    let _ = j.emit(trace, None, None, payload);
}

/// CV3: one turn's streaming sink. Chunks go to the chunk player
/// the moment they land (the player settles the ack before the
/// first one, so a fired ack never overlaps answer audio); a
/// preamble mark means the chunks were not the answer, and the
/// caller speaks the answer whole too.
struct TurnSink {
    ack: Option<crate::ack::AckWatch>,
    player: Option<crate::stream::ChunkPlayer>,
    config: Config,
    out_wav: String,
    chunks: usize,
    preamble: bool,
}

impl agentd::ChunkSink for TurnSink {
    fn on_chunk(&mut self, text: &str) {
        if self.player.is_none() {
            let ack = self.ack.take();
            self.player = Some(crate::stream::ChunkPlayer::start(&self.config, &self.out_wav, ack));
        }
        if let Some(player) = &self.player {
            player.push(text);
        }
        self.chunks += 1;
    }

    fn on_preamble(&mut self) {
        self.preamble = true;
    }
}

/// VoiceError carries only strings and numbers, but does not derive
/// Clone; the request thread needs to send its failure across a
/// channel while keeping its own copy for its return value.
fn clone_voice_error(e: &VoiceError) -> VoiceError {
    match e {
        VoiceError::Config(s) => VoiceError::Config(s.clone()),
        VoiceError::MicBusy { holder, lock } => {
            VoiceError::MicBusy { holder: holder.clone(), lock: lock.clone() }
        }
        VoiceError::MicOff { state_file } => VoiceError::MicOff { state_file: state_file.clone() },
        VoiceError::Capture(s) => VoiceError::Capture(s.clone()),
        VoiceError::SttUnavailable { engine, detail } => {
            VoiceError::SttUnavailable { engine: engine.clone(), detail: detail.clone() }
        }
        VoiceError::EmptyTranscript { engine } => {
            VoiceError::EmptyTranscript { engine: engine.clone() }
        }
        VoiceError::AgentUnreachable { endpoint, detail } => {
            VoiceError::AgentUnreachable { endpoint: endpoint.clone(), detail: detail.clone() }
        }
        VoiceError::AgentRefused { endpoint, status, detail } => VoiceError::AgentRefused {
            endpoint: endpoint.clone(),
            status: *status,
            detail: detail.clone(),
        },
        VoiceError::NoAnswer { endpoint, detail } => {
            VoiceError::NoAnswer { endpoint: endpoint.clone(), detail: detail.clone() }
        }
        VoiceError::TtsFailed { engine, detail } => {
            VoiceError::TtsFailed { engine: engine.clone(), detail: detail.clone() }
        }
        VoiceError::PlaybackRefused { reason } => {
            VoiceError::PlaybackRefused { reason: reason.clone() }
        }
        VoiceError::Io { op, detail } => {
            VoiceError::Io { op: op.clone(), detail: detail.clone() }
        }
    }
}

/// The whole voice turn. Any failure leaves a named record behind and returns
/// the error; nothing here fails silently.
pub fn run(cfg: &Config, req: &Request, j: &mut JournalOut) -> Result<Outcome, VoiceError> {
    let started = Instant::now();
    let endpoint = agentd::Endpoint::parse(&cfg.agentd)?;

    // The user's mic switch is checked before anything opens: while it
    // says off, voiced refuses capture outright, and the journal shows
    // the refusal instead of a capture that never should have started.
    if capture::mic_state(Path::new(&cfg.mic_state_file))? == capture::MicState::Off {
        let err = VoiceError::MicOff { state_file: cfg.mic_state_file.clone() };
        emit(j, Payload::Op6(Op6Event::PowerChange { component: "mic".into(), state: PowerState::Off, reason: "mic off by user: voiced refused capture".into() }));
        emit(j, Payload::AgUi(AgUiEvent::RunError { message: err.to_string(), code: Some("mic_off".into()), usage: vec![] }));
        return Err(err);
    }

    // Mic ownership: one supervised holder, and the journal says who.
    let _mic = capture::acquire_mic(Path::new(&cfg.lock), &format!("{} pid {}", SERVICE_NAME, std::process::id()))?;
    emit(j, Payload::Op6(Op6Event::PowerChange { component: "mic".into(), state: PowerState::On, reason: "voiced opened capture for a voice request".into() }));
    let deadline = (started.elapsed().as_micros() as u64) + 15_000_000;
    emit(j, Payload::Op6(Op6Event::WakeLockAcquired { holder: SERVICE_NAME.into(), deadline_us: Some(deadline), max_us: Some(15_000_000) }));

    let pcm = match &req.input {
        Input::Wav { path } => capture::capture_wav(path, j)?,
        Input::Mic { cmd, seconds } => {
            // CV2: a configured Smart Turn judge advises the live
            // capture's endpointing; without one the CV0 endpointer
            // rules alone, exactly as before.
            let judge = cfg.smart_turn_cmd.as_deref().map(crate::turn::TurnJudge::new);
            capture::capture_mic(cmd, *seconds, judge.as_ref(), j)?
        }
    };
    emit(j, Payload::Op6(Op6Event::PowerChange { component: "mic".into(), state: PowerState::Off, reason: "capture finished, microphone released".into() }));

    let (heard, asr_confidence) = match stt::transcribe(cfg, &pcm, j) {
        Ok(t) => t,
        Err(e) => {
            emit(j, Payload::AgUi(AgUiEvent::RunError { message: e.to_string(), code: Some("stt_failed".into()), usage: vec![] }));
            return Err(e);
        }
    };
    // CV0 instant ack: the watch starts at the STT final. It settles
    // (and is journalled, fired or not) on every path out of the
    // answer wait - on success just before playback, so a fired ack
    // never overlaps the answer, and on each failure return too.
    let stt_final_at = Instant::now();
    let cleaned = stt::clean(cfg, &heard, j);
    let mut ack = crate::ack::AckWatch::start(cfg, stt_final_at);

    let after = agentd::journal_last_seq(&endpoint);

    // CV3: mint the run id, start the tail BEFORE the POST returns
    // (so none of the run's frames can slip past), and let the POST
    // resolve on its own thread while the tail streams. Sentences
    // the brain completes mid-run are journaled by agentd as
    // answer_chunk records; the sink speaks each one as it lands —
    // the serial wait for the whole answer becomes the wait for its
    // first sentence.
    let minted = agentd::mint_run_id();
    let (signal_tx, signal_rx) = std::sync::mpsc::channel::<agentd::PostSignal>();
    let post_endpoint = endpoint.clone();
    let post_text = cleaned.clone();
    let post_session = req.session.clone();
    let post_follow_up = req.follow_up;
    let post_timeout = req.answer_timeout;
    let post_run_id = minted.clone();
    let post_handle = std::thread::spawn(move || {
        let mut post_journal = crate::stream::thread_journal();
        let result = agentd::post_request_session_rid(
            &post_endpoint,
            &post_text,
            post_session.as_deref(),
            post_follow_up,
            asr_confidence,
            Some(&post_run_id),
            post_timeout,
            &mut post_journal,
        );
        match &result {
            Ok(id) => {
                let _ = signal_tx.send(agentd::PostSignal::Adopted(id.clone()));
            }
            Err(e) => {
                let _ = signal_tx.send(agentd::PostSignal::Failed(clone_voice_error(e)));
            }
        }
        (result, post_journal.events())
    });

    let mut sink = TurnSink {
        ack,
        player: None,
        config: cfg.clone(),
        out_wav: req.out_wav.clone(),
        chunks: 0,
        preamble: false,
    };
    let tail_result = agentd::tail_answer_streaming(
        &endpoint,
        &cleaned,
        &minted,
        j,
        req.answer_timeout,
        after,
        &signal_rx,
        &mut sink,
    );
    // The request thread has resolved or nearly so on every path;
    // collect it and merge its journal before the turn's own closes.
    match post_handle.join() {
        Ok((_post_result, post_events)) => crate::stream::merge_journal(j, post_events),
        Err(_) => {}
    }

    let answer = match tail_result {
        Ok(a) => a,
        Err(e) => {
            // Speech may already have happened (chunks played before
            // the failure). Let the player finish what was queued —
            // partial speech stays honest — then fail the turn.
            if let Some(player) = sink.player.take() {
                let _ = player.finish(j);
            }
            emit(j, Payload::AgUi(AgUiEvent::RunError { message: e.to_string(), code: Some("no_answer".into()), usage: vec![] }));
            if let Some(w) = sink.ack.take() { w.settle(j); }
            return Err(e);
        }
    };

    let chunks_played = match sink.player.take() {
        Some(player) => match player.finish(j) {
            Ok(n) => n,
            Err(e) => {
                emit(j, Payload::AgUi(AgUiEvent::RunError { message: e.to_string(), code: Some("tts_failed".into()), usage: vec![] }));
                return Err(e);
            }
        },
        None => 0,
    };

    if chunks_played == 0 || sink.preamble {
        // No chunks arrived (a buffered brain or a pre-CV3 agentd),
        // or the chunks were a preamble and the answer itself has
        // not been spoken yet: the classic whole-answer synthesis.
        if let Err(e) = tts::synthesize(cfg, &answer, &req.out_wav, j) {
            emit(j, Payload::AgUi(AgUiEvent::RunError { message: e.to_string(), code: Some("tts_failed".into()), usage: vec![] }));
            if let Some(w) = sink.ack.take() { w.settle(j); }
            return Err(e);
        }
        if let Some(w) = sink.ack.take() { w.settle(j); }
        tts::play(cfg, &req.out_wav, j)?;
    }

    emit(j, Payload::Op6(Op6Event::WakeLockReleased { holder: SERVICE_NAME.into(), reason: Some(format!("voice turn done in {} ms", started.elapsed().as_millis())) }));

    Ok(Outcome { heard, cleaned, answer, out_wav: req.out_wav.clone() })
}
