//! voiced CLI. The journal goes to stdout (or `--journal PATH`); every human
//! line goes to stderr, so the journal stays parseable.

use std::time::Duration;

use voiced::config::Config;
use voiced::error::VoiceError;
use voiced::{run, Input, Request, SERVICE_NAME};

const USAGE: &str = "\
voiced — capture -> STT -> agentd -> TTS, every step journalled

  voiced --wav IN.WAV [--config CONF] [--out ANSWER.WAV] [--journal PATH]
  voiced --mic 'arecord -q -f S16_LE -r 16000 -c 1 -t raw' --seconds 8
  voiced --wav IN.WAV --stt 'wyoming 127.0.0.1:10300' --tts 'cmd /usr/local/bin/say.sh'

  --config PATH     key = value config (stt, tts, agentd, play, quiet, lock, mic_state, clean, ack_clips, ack_delay_ms)
  --mic-state PATH  override the mic switch file (one word: on | off)
  --wav PATH        recorded input (the only input an automated run may use)
  --mic CMD         live capture command, raw s16le mono 16 kHz on stdout
  --seconds N       capture limit with --mic (default 8)
  --out PATH        where the spoken answer is written (default /tmp/voiced-answer.wav)
  --journal PATH    write the journal here instead of stdout
  --agentd URL      override agentd base URL
  --stt SPEC        override the STT engine (wyoming host:port | cmd <cmd> | stub <text>)
  --tts SPEC        override the TTS engine (wyoming host:port | cmd <cmd> | none)
  --session ID      CV1: the conversation session this turn belongs to
  --follow-up       CV1: this turn was accepted without the wake word
  --quiet           synthesise but do not play (default)
  --loud            play the answer
  --timeout N       seconds to wait for the answer (default 30)
";

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("{USAGE}");
        std::process::exit(0);
    }
    match real_main(&args) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("voiced: {e}");
            std::process::exit(1);
        }
    }
}

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn real_main(args: &[String]) -> Result<(), VoiceError> {
    let mut cfg = match opt(args, "--config") {
        Some(p) => Config::load(std::path::Path::new(&p))?,
        None => Config::default(),
    };
    if let Some(v) = opt(args, "--agentd") {
        if !v.starts_with("http://") {
            return Err(VoiceError::Config(format!("--agentd must be an http:// URL, got {v:?}")));
        }
        cfg.agentd = v.trim_end_matches('/').into();
    }
    if let Some(v) = opt(args, "--stt") {
        cfg.stt = match v.split_once(' ') {
            Some(("cmd", rest)) if !rest.trim().is_empty() => voiced::config::SttEngine::Cmd { cmd: rest.trim().into() },
            Some(("wyoming", rest)) if !rest.trim().is_empty() => voiced::config::SttEngine::Wyoming { destination: rest.trim().into() },
            Some(("stub", rest)) => voiced::config::SttEngine::Stub { text: rest.to_string() },
            other => return Err(VoiceError::Config(format!("--stt must be 'wyoming host:port', 'cmd <cmd>' or 'stub <text>', got {other:?}"))),
        };
    }
    if let Some(v) = opt(args, "--tts") {
        cfg.tts = match v.split_once(' ') {
            Some(("cmd", rest)) if !rest.trim().is_empty() => voiced::config::TtsEngine::Cmd { cmd: rest.trim().into() },
            Some(("wyoming", rest)) if !rest.trim().is_empty() => voiced::config::TtsEngine::Wyoming { destination: rest.trim().into() },
            Some(("none", _)) => voiced::config::TtsEngine::None,
            other => return Err(VoiceError::Config(format!("--tts must be 'wyoming host:port', 'cmd <cmd>' or 'none', got {other:?}"))),
        };
    }
    if flag(args, "--loud") {
        cfg.quiet = false;
    }
    if let Some(v) = opt(args, "--mic-state") {
        cfg.mic_state_file = v;
    }

    let input = match (opt(args, "--wav"), opt(args, "--mic")) {
        (Some(p), None) => Input::Wav { path: p },
        (None, Some(cmd)) => Input::Mic { cmd, seconds: opt(args, "--seconds").unwrap_or_else(|| "8".into()).parse::<u64>().map_err(|_| VoiceError::Config("--seconds must be a number".into()))? },
        (Some(_), Some(_)) => return Err(VoiceError::Config("give either --wav or --mic, not both".into())),
        (None, None) => return Err(VoiceError::Config("no input: pass --wav IN.WAV or --mic CMD".into())),
    };
    let out_wav = opt(args, "--out").unwrap_or_else(|| "/tmp/voiced-answer.wav".into());
    let timeout = opt(args, "--timeout").unwrap_or_else(|| "30".into()).parse::<u64>().map_err(|_| VoiceError::Config("--timeout must be a number of seconds".into()))?;

    let sink: Box<dyn std::io::Write> = match opt(args, "--journal") {
        Some(p) => Box::new(std::fs::File::create(&p).map_err(|e| VoiceError::Io { op: format!("create journal {p}"), detail: e.to_string() })?),
        None => Box::new(std::io::stdout()),
    };
    let mut journal = voiced::capture::JournalOut::with_service(sink, SERVICE_NAME.into());

    let req = Request {
        input,
        out_wav,
        answer_timeout: Duration::from_secs(timeout),
        session: opt(args, "--session"),
        follow_up: flag(args, "--follow-up"),
    };
    let outcome = run(&cfg, &req, &mut journal)?;
    eprintln!("voiced: heard {:?} -> answer {:?} -> {}", outcome.cleaned, outcome.answer, outcome.out_wav);
    Ok(())
}
