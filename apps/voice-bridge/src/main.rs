//! Thin CLI wrapper: parse args, connect, run the bridge. All output is
//! trace-events JSONL on stdout; a one-line human summary goes to stderr.
//! Any failure — bad WAV, refused connection, mid-stream close — is also
//! journaled as an explicit error event before exiting non-zero.

use std::io::BufReader;
use std::net::TcpStream;
use std::process::ExitCode;

use trace_events::{AgUiEvent, HandoffStatus, Op6Event, Payload};
use voice_bridge::cli::{parse_args, Command, USAGE};
use voice_bridge::error::BridgeError;
use voice_bridge::journal::Journal;
use voice_bridge::stt::{run_stt, SttOptions};
use voice_bridge::tts::{run_tts, TtsOptions};
use voice_bridge::wav::read_pcm;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match parse_args(&args) {
        Ok(Some(cmd)) => cmd,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let mut journal = Journal::new(std::io::stdout());
    let result = match &cmd {
        Command::Stt { stt_addr, wav_path, language } => {
            stt(&mut journal, stt_addr, wav_path, language.as_deref())
        }
        Command::Tts { tts_addr, text, out_path, voice, language } => {
            tts(&mut journal, tts_addr, text, out_path, voice.as_deref(), language.as_deref())
        }
    };
    match result {
        Ok(summary) => {
            eprintln!("{summary}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn stt(
    journal: &mut Journal<std::io::Stdout>,
    stt_addr: &str,
    wav_path: &str,
    language: Option<&str>,
) -> Result<String, BridgeError> {
    // Preflight failures still get explicit journal events — never silent.
    let pcm = match read_pcm(wav_path) {
        Ok(pcm) => pcm,
        Err(msg) => return Err(preflight_fail(journal, stt_addr, msg)),
    };
    if pcm.rate != 16000 || pcm.channels != 1 {
        return Err(preflight_fail(
            journal,
            stt_addr,
            format!(
                "{} is {} Hz x {} ch; voice-bridge streams 16 kHz mono WAVs",
                wav_path, pcm.rate, pcm.channels
            ),
        ));
    }
    let mut stream = match TcpStream::connect(stt_addr) {
        Ok(s) => s,
        Err(e) => {
            return Err(preflight_fail(
                journal,
                stt_addr,
                format!("cannot reach wyoming stt at {stt_addr}: {e}"),
            ))
        }
    };
    let mut writer = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            return Err(preflight_fail(
                journal,
                stt_addr,
                format!("cannot duplicate stt socket: {e}"),
            ))
        }
    };
    let mut reader = BufReader::new(&mut stream);
    let opts = SttOptions { destination: stt_addr, source: wav_path, language };
    let text = run_stt(&mut reader, &mut writer, journal, &pcm, &opts)?;
    Ok(format!("transcript: {text}"))
}

fn tts(
    journal: &mut Journal<std::io::Stdout>,
    tts_addr: &str,
    text: &str,
    out_path: &str,
    voice: Option<&str>,
    language: Option<&str>,
) -> Result<String, BridgeError> {
    let mut stream = match TcpStream::connect(tts_addr) {
        Ok(s) => s,
        Err(e) => {
            return Err(preflight_fail(
                journal,
                tts_addr,
                format!("cannot reach wyoming tts at {tts_addr}: {e}"),
            ))
        }
    };
    let mut writer = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            return Err(preflight_fail(
                journal,
                tts_addr,
                format!("cannot duplicate tts socket: {e}"),
            ))
        }
    };
    let mut reader = BufReader::new(&mut stream);
    let opts = TtsOptions { destination: tts_addr, text, voice, language, out_path };
    let result = run_tts(&mut reader, &mut writer, journal, &opts)?;
    Ok(format!(
        "wrote {out_path} ({} bytes, {} Hz, {} ms to first audio)",
        result.bytes,
        result.rate,
        result.time_to_first_audio_us / 1000
    ))
}

/// Journal a failure that happened before the protocol exchange started
/// (bad input, refused connection), then return it as an error.
fn preflight_fail(
    journal: &mut Journal<std::io::Stdout>,
    destination: &str,
    message: String,
) -> BridgeError {
    let trace = journal.new_trace();
    let _ = journal.emit(
        trace,
        None,
        None,
        Payload::Op6(Op6Event::Handoff {
            destination: destination.to_string(),
            reason: message.clone(),
            status: HandoffStatus::Started,
        }),
    );
    let _ = journal.emit(
        trace,
        None,
        None,
        Payload::AgUi(AgUiEvent::RunError {
            message: message.clone(),
            code: None,
            usage: Vec::new(),
        }),
    );
    let _ = journal.emit(
        trace,
        None,
        None,
        Payload::Op6(Op6Event::Handoff {
            destination: destination.to_string(),
            reason: message.clone(),
            status: HandoffStatus::Failed,
        }),
    );
    BridgeError::Protocol(message)
}
