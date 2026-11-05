//! Hand-rolled argument parsing (no clap dependency — the task asks for
//! minimal deps). Two modes:
//!
//! ```text
//! voice-bridge --stt HOST:PORT --wav IN.WAV [--language LANG]
//! voice-bridge --tts HOST:PORT --text TEXT [--out OUT.WAV] [--voice NAME] [--language LANG]
//! ```

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Stt {
        stt_addr: String,
        wav_path: String,
        language: Option<String>,
    },
    Tts {
        tts_addr: String,
        text: String,
        out_path: String,
        voice: Option<String>,
        language: Option<String>,
    },
}

pub const USAGE: &str = "\
usage: voice-bridge --stt HOST:PORT --wav IN.WAV [--language LANG]
       voice-bridge --tts HOST:PORT --text TEXT [--out OUT.WAV] [--voice NAME] [--language LANG]

Streams a 16 kHz mono PCM16 WAV to a Wyoming STT server, or text to a
Wyoming TTS server, emitting every step as trace-events JSONL on stdout.";

/// Consume `--flag VALUE` at `args[i]`, advancing `i` past the value.
fn take_value(
    args: &[String],
    i: &mut usize,
    flag: &str,
    slot: &mut Option<String>,
) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("{flag} given more than once"));
    }
    let value = args
        .get(*i + 1)
        .ok_or_else(|| format!("{flag} needs a value"))?;
    *slot = Some(value.clone());
    *i += 1;
    Ok(())
}

/// Parse CLI arguments (without the program name).
/// `Ok(None)` means help was requested and printed by the caller.
pub fn parse_args(args: &[String]) -> Result<Option<Command>, String> {
    let mut stt: Option<String> = None;
    let mut tts: Option<String> = None;
    let mut wav: Option<String> = None;
    let mut text: Option<String> = None;
    let mut out: Option<String> = None;
    let mut voice: Option<String> = None;
    let mut language: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => return Ok(None),
            "--stt" => take_value(args, &mut i, "--stt", &mut stt)?,
            "--tts" => take_value(args, &mut i, "--tts", &mut tts)?,
            "--wav" => take_value(args, &mut i, "--wav", &mut wav)?,
            "--text" => take_value(args, &mut i, "--text", &mut text)?,
            "--out" => take_value(args, &mut i, "--out", &mut out)?,
            "--voice" => take_value(args, &mut i, "--voice", &mut voice)?,
            "--language" | "-l" => take_value(args, &mut i, "--language", &mut language)?,
            other => return Err(format!("unknown argument {other}")),
        }
        i += 1;
    }

    match (stt, tts) {
        (Some(_), Some(_)) => Err("use either --stt or --tts, not both".into()),
        (None, None) => Err("one of --stt or --tts is required".into()),
        (Some(stt_addr), None) => {
            let wav_path = wav.ok_or_else(|| "--stt requires --wav".to_string())?;
            Ok(Some(Command::Stt {
                stt_addr,
                wav_path,
                language,
            }))
        }
        (None, Some(tts_addr)) => {
            let text = text.ok_or_else(|| "--tts requires --text".to_string())?;
            Ok(Some(Command::Tts {
                tts_addr,
                text,
                out_path: out.unwrap_or_else(|| "tts-out.wav".to_string()),
                voice,
                language,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_stt_mode() {
        let cmd = parse_args(&args("--stt 127.0.0.1:10300 --wav in.wav --language en")).unwrap();
        assert_eq!(
            cmd,
            Some(Command::Stt {
                stt_addr: "127.0.0.1:10300".into(),
                wav_path: "in.wav".into(),
                language: Some("en".into()),
            })
        );
    }

    #[test]
    fn parses_tts_mode_with_defaults() {
        let cmd = parse_args(&args("--tts host:1 --text hello")).unwrap();
        assert_eq!(
            cmd,
            Some(Command::Tts {
                tts_addr: "host:1".into(),
                text: "hello".into(),
                out_path: "tts-out.wav".into(),
                voice: None,
                language: None,
            })
        );
    }

    #[test]
    fn rejects_bad_combinations() {
        assert!(parse_args(&args("--wav in.wav")).unwrap_err().contains("--stt or --tts"));
        assert!(parse_args(&args("--stt h:1 --tts h:2 --wav a")).unwrap_err().contains("not both"));
        assert!(parse_args(&args("--stt h:1")).unwrap_err().contains("requires --wav"));
        assert!(parse_args(&args("--tts h:1")).unwrap_err().contains("requires --text"));
        assert!(parse_args(&args("--stt h:1 --wav a --wav b")).unwrap_err().contains("more than once"));
        assert!(parse_args(&args("--stt")).unwrap_err().contains("needs a value"));
        assert!(parse_args(&args("--bogus")).unwrap_err().contains("unknown"));
    }

    #[test]
    fn help_returns_none() {
        assert_eq!(parse_args(&args("--help")).unwrap(), None);
        assert_eq!(parse_args(&args("-h")).unwrap(), None);
    }
}
