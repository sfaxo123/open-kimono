//! Round-trip and wire-format tests for the typed events.
//!
//! Wire-format assertions pin the exact JSON `data` object (key order
//! included) that the Python reference produces with `json.dumps`, so the
//! Rust encoder is byte-compatible with the reference.

use std::collections::VecDeque;
use std::io::Cursor;

use serde_json::{json, Map};
use wyoming::events::asr::*;
use wyoming::events::audio::*;
use wyoming::events::error::ErrorEvent;
use wyoming::events::handle::*;
use wyoming::events::info::*;
use wyoming::events::intent::*;
use wyoming::events::ping::*;
use wyoming::events::pipeline::*;
use wyoming::events::satellite::*;
use wyoming::events::snd::Played;
use wyoming::events::timer::*;
use wyoming::events::tts::*;
use wyoming::events::user_event::UserEvent;
use wyoming::events::vad::*;
use wyoming::events::wake::*;
use wyoming::events::{EventKind, EventKindError, Eventable};
use wyoming::{decode, encode, Event};

/// Serialize an event's `data` object to a compact JSON string (key order
/// preserved, like Python `json.dumps(separators=...)`).
fn data_json(event: &Event) -> String {
    serde_json::to_string(&event.data).unwrap()
}

/// Round-trip a typed event through `to_event`/`from_event`.
fn roundtrip<T: Eventable + std::fmt::Debug + PartialEq>(value: &T) {
    let event = value.to_event();
    let back = T::from_event(&event).expect("from_event should succeed");
    assert_eq!(&back, value);
}

/// Round-trip a typed event through the full frame codec.
fn frame_roundtrip<T: Eventable + std::fmt::Debug + PartialEq>(value: &T) {
    let bytes = encode(&value.to_event());
    let mut cursor = Cursor::new(bytes);
    let event = decode(&mut cursor).unwrap().expect("one event expected");
    let back = T::from_event(&event).expect("from_event should succeed");
    assert_eq!(&back, value);
}

// ---------------------------------------------------------------- audio

#[test]
fn audio_chunk_wire_format_and_roundtrip() {
    let chunk = AudioChunk {
        rate: 16000,
        width: 2,
        channels: 1,
        audio: vec![1, 2, 3, 4],
        timestamp: None,
    };
    let event = chunk.to_event();
    assert_eq!(event.type_, "audio-chunk");
    assert_eq!(
        data_json(&event),
        r#"{"rate":16000,"width":2,"channels":1,"timestamp":null}"#
    );
    assert_eq!(event.payload.as_deref(), Some(&chunk.audio[..]));
    roundtrip(&chunk);
    frame_roundtrip(&chunk);
}

#[test]
fn audio_start_wire_order_and_roundtrip() {
    let start = AudioStart {
        rate: 22050,
        width: 2,
        channels: 2,
        timestamp: Some(1234),
    };
    let event = start.to_event();
    assert_eq!(event.type_, "audio-start");
    assert_eq!(
        data_json(&event),
        r#"{"rate":22050,"width":2,"channels":2,"timestamp":1234}"#
    );
    roundtrip(&start);
}

#[test]
fn audio_stop_always_includes_timestamp() {
    let stop = AudioStop { timestamp: None };
    assert_eq!(data_json(&stop.to_event()), r#"{"timestamp":null}"#);
    roundtrip(&stop);
    let stop2 = AudioStop {
        timestamp: Some(99),
    };
    assert_eq!(data_json(&stop2.to_event()), r#"{"timestamp":99}"#);
    roundtrip(&stop2);
}

// ---------------------------------------------------------------- vad

#[test]
fn voice_started_stopped() {
    let started = VoiceStarted { timestamp: None };
    assert_eq!(data_json(&started.to_event()), r#"{"timestamp":null}"#);
    roundtrip(&started);
    let stopped = VoiceStopped {
        timestamp: Some(500),
    };
    assert_eq!(data_json(&stopped.to_event()), r#"{"timestamp":500}"#);
    roundtrip(&stopped);
}

// ---------------------------------------------------------------- ping

#[test]
fn ping_pong() {
    let ping = Ping { text: None };
    assert_eq!(data_json(&ping.to_event()), r#"{"text":null}"#);
    roundtrip(&ping);
    let pong = Pong {
        text: Some("pong".to_string()),
    };
    assert_eq!(data_json(&pong.to_event()), r#"{"text":"pong"}"#);
    roundtrip(&pong);
}

// ---------------------------------------------------------------- error

#[test]
fn error_event_omits_missing_code() {
    let err = ErrorEvent {
        text: "something broke".to_string(),
        code: None,
    };
    assert_eq!(
        data_json(&err.to_event()),
        r#"{"text":"something broke"}"#
    );
    roundtrip(&err);
    let err2 = ErrorEvent {
        text: "no".to_string(),
        code: Some("bad-request".to_string()),
    };
    assert_eq!(
        data_json(&err2.to_event()),
        r#"{"text":"no","code":"bad-request"}"#
    );
    roundtrip(&err2);
}

// ---------------------------------------------------------------- timer

#[test]
fn timer_started_minimal_and_full() {
    let minimal = TimerStarted {
        id: "timer-1".to_string(),
        name: None,
        total_seconds: 60,
        start_hours: None,
        start_minutes: None,
        start_seconds: None,
    };
    assert_eq!(
        data_json(&minimal.to_event()),
        r#"{"id":"timer-1","total_seconds":60}"#
    );
    roundtrip(&minimal);

    let full = TimerStarted {
        id: "timer-2".to_string(),
        name: Some("kitchen".to_string()),
        total_seconds: 3725,
        start_hours: Some(1),
        start_minutes: Some(2),
        start_seconds: Some(5),
    };
    assert_eq!(
        data_json(&full.to_event()),
        r#"{"id":"timer-2","total_seconds":3725,"name":"kitchen","start_hours":1,"start_minutes":2,"start_seconds":5}"#
    );
    roundtrip(&full);
    frame_roundtrip(&full);
}

#[test]
fn timer_updated_cancelled_finished() {
    let updated = TimerUpdated {
        id: "t".to_string(),
        is_active: true,
        total_seconds: 3,
    };
    assert_eq!(
        data_json(&updated.to_event()),
        r#"{"id":"t","is_active":true,"total_seconds":3}"#
    );
    roundtrip(&updated);
    let cancelled = TimerCancelled { id: "t".to_string() };
    assert_eq!(data_json(&cancelled.to_event()), r#"{"id":"t"}"#);
    roundtrip(&cancelled);
    let finished = TimerFinished { id: "t".to_string() };
    assert_eq!(data_json(&finished.to_event()), r#"{"id":"t"}"#);
    roundtrip(&finished);
}

// ---------------------------------------------------------------- user event

#[test]
fn user_event_omits_empty_data_context() {
    let bare = UserEvent {
        name: "my-event".to_string(),
        data: None,
        context: None,
    };
    assert_eq!(data_json(&bare.to_event()), r#"{"name":"my-event"}"#);
    roundtrip(&bare);

    let mut data = Map::new();
    data.insert("on".to_string(), json!(true));
    let full = UserEvent {
        name: "light".to_string(),
        data: Some(data),
        context: None,
    };
    assert_eq!(data_json(&full.to_event()), r#"{"name":"light","data":{"on":true}}"#);
    roundtrip(&full);
}

// ---------------------------------------------------------------- snd

#[test]
fn played_has_no_data() {
    let played = Played;
    let event = played.to_event();
    assert_eq!(event.type_, "played");
    assert!(event.data.is_empty());
    assert_eq!(event.payload, None);
    roundtrip(&played);
}

// ---------------------------------------------------------------- asr

#[test]
fn transcript_wire_order_and_roundtrip() {
    let mut context = Map::new();
    context.insert("session".to_string(), json!("abc"));
    let t = Transcript {
        text: "turn on the lights".to_string(),
        language: Some("en".to_string()),
        context: Some(context),
    };
    assert_eq!(
        data_json(&t.to_event()),
        r#"{"text":"turn on the lights","language":"en","context":{"session":"abc"}}"#
    );
    roundtrip(&t);
    frame_roundtrip(&t);
}

#[test]
fn transcribe_full_wire_order() {
    let tr = Transcribe {
        name: Some("whisper".to_string()),
        language: Some("en".to_string()),
        context: None,
        vad_sensitivity: Some(VadSensitivity::Aggressive),
        transcript_names: Some(vec!["jordan".to_string()]),
        transcript_terms: Some(vec!["laptop".to_string()]),
    };
    assert_eq!(
        data_json(&tr.to_event()),
        r#"{"name":"whisper","language":"en","vad_sensitivity":"aggressive","transcript_names":["jordan"],"transcript_terms":["laptop"]}"#
    );
    roundtrip(&tr);
}

#[test]
fn vad_sensitivity_custom_passthrough() {
    let tr = Transcribe {
        vad_sensitivity: Some(VadSensitivity::Custom("0.75".to_string())),
        ..Default::default()
    };
    assert_eq!(
        data_json(&tr.to_event()),
        r#"{"vad_sensitivity":"0.75"}"#
    );
    roundtrip(&tr);
}

#[test]
fn transcript_start_tolerates_empty_data() {
    let event = Event::new("transcript-start");
    let start = TranscriptStart::from_event(&event).expect("empty data is valid");
    assert_eq!(start.language, None);
    assert_eq!(start.context, None);
}

#[test]
fn transcript_stream_events() {
    roundtrip(&TranscriptChunk {
        text: "hel".to_string(),
    });
    roundtrip(&TranscriptStop);
    let stop = TranscriptStop.to_event();
    assert_eq!(stop.type_, "transcript-stop");
    assert!(stop.data.is_empty());
}

// ---------------------------------------------------------------- wake

#[test]
fn detection_always_includes_name_timestamp_speaker() {
    let d = Detection {
        name: None,
        timestamp: None,
        speaker: None,
        context: None,
    };
    assert_eq!(
        data_json(&d.to_event()),
        r#"{"name":null,"timestamp":null,"speaker":null}"#
    );
    roundtrip(&d);

    let full = Detection {
        name: Some("hey mycroft".to_string()),
        timestamp: Some(1700000000),
        speaker: Some("jordan".to_string()),
        context: Some(Map::from_iter([("score".to_string(), json!(0.9))])),
    };
    assert_eq!(
        data_json(&full.to_event()),
        r#"{"name":"hey mycroft","timestamp":1700000000,"speaker":"jordan","context":{"score":0.9}}"#
    );
    roundtrip(&full);
    frame_roundtrip(&full);
}

#[test]
fn detect_names_always_included() {
    let d = Detect {
        names: None,
        context: None,
    };
    assert_eq!(data_json(&d.to_event()), r#"{"names":null}"#);
    roundtrip(&d);

    let with = Detect {
        names: Some(vec!["hey mycroft".to_string()]),
        context: None,
    };
    assert_eq!(
        data_json(&with.to_event()),
        r#"{"names":["hey mycroft"]}"#
    );
    roundtrip(&with);
}

#[test]
fn not_detected() {
    roundtrip(&NotDetected { context: None });
}

// ---------------------------------------------------------------- tts

#[test]
fn synthesize_full_wire_order_voice_name_wins() {
    let s = Synthesize {
        text: "hello world".to_string(),
        voice: Some(SynthesizeVoice {
            name: Some("piper".to_string()),
            language: Some("en".to_string()), // dropped: name wins
            speaker: Some("en_US-lessac-medium".to_string()),
        }),
        text_format: Some(SynthesizeTextFormat::Ssml),
        context: Some(Map::from_iter([("k".to_string(), json!(1))])),
    };
    assert_eq!(
        data_json(&s.to_event()),
        r#"{"text":"hello world","voice":{"name":"piper","speaker":"en_US-lessac-medium"},"text_format":"ssml","context":{"k":1}}"#
    );
    // name wins: language is lost on the wire, so the round-trip value has
    // language = None (same as the Python reference).
    let event = s.to_event();
    let back = Synthesize::from_event(&event).unwrap();
    assert_eq!(
        back.voice,
        Some(SynthesizeVoice {
            name: Some("piper".to_string()),
            language: None,
            speaker: Some("en_US-lessac-medium".to_string()),
        })
    );
}

#[test]
fn synthesize_voice_language_only() {
    let s = Synthesize {
        text: "hi".to_string(),
        voice: Some(SynthesizeVoice {
            name: None,
            language: Some("de".to_string()),
            speaker: Some("ignored".to_string()), // dropped without name
        }),
        text_format: None,
        context: None,
    };
    assert_eq!(
        data_json(&s.to_event()),
        r#"{"text":"hi","voice":{"language":"de"}}"#
    );
    let back = Synthesize::from_event(&s.to_event()).unwrap();
    assert_eq!(back.voice.as_ref().unwrap().language.as_deref(), Some("de"));
}

#[test]
fn synthesize_empty_voice_becomes_none() {
    let mut data = Map::new();
    data.insert("text".to_string(), json!("hi"));
    data.insert("voice".to_string(), json!({}));
    let event = Event {
        type_: "synthesize".to_string(),
        data,
        payload: None,
    };
    let back = Synthesize::from_event(&event).unwrap();
    assert_eq!(back.voice, None);
}

#[test]
fn synthesize_text_format_custom_passthrough() {
    let s = Synthesize {
        text: "x".to_string(),
        voice: None,
        text_format: Some(SynthesizeTextFormat::Custom("markdown".to_string())),
        context: None,
    };
    assert_eq!(
        data_json(&s.to_event()),
        r#"{"text":"x","text_format":"markdown"}"#
    );
    roundtrip(&s);
}

#[test]
fn synthesize_stream_events() {
    roundtrip(&SynthesizeStart {
        voice: Some(SynthesizeVoice {
            name: Some("v".to_string()),
            language: None,
            speaker: None,
        }),
        text_format: Some(SynthesizeTextFormat::Text),
        context: None,
    });
    roundtrip(&SynthesizeChunk {
        text: "chunk".to_string(),
    });
    roundtrip(&SynthesizeStop);
    roundtrip(&SynthesizeStopped);
    assert!(SynthesizeStop.to_event().data.is_empty());
    assert!(SynthesizeStopped.to_event().data.is_empty());
}

// ---------------------------------------------------------------- intent

#[test]
fn recognize_roundtrip() {
    roundtrip(&Recognize {
        text: "turn on the lights".to_string(),
        context: Some(Map::from_iter([("room".to_string(), json!("office"))])),
    });
}

#[test]
fn intent_omits_empty_entities() {
    let empty = Intent {
        name: "turn.on".to_string(),
        entities: Vec::new(),
        text: None,
        context: None,
    };
    assert_eq!(data_json(&empty.to_event()), r#"{"name":"turn.on"}"#);
    roundtrip(&empty);

    let full = Intent {
        name: "turn.on".to_string(),
        entities: vec![
            Entity {
                name: "light".to_string(),
                value: Some(json!("desk lamp")),
            },
            Entity {
                name: "brightness".to_string(),
                value: None,
            },
        ],
        text: Some("Okay".to_string()),
        context: None,
    };
    assert_eq!(
        data_json(&full.to_event()),
        r#"{"name":"turn.on","entities":[{"name":"light","value":"desk lamp"},{"name":"brightness","value":null}],"text":"Okay"}"#
    );
    roundtrip(&full);
    frame_roundtrip(&full);
}

#[test]
fn intent_to_rhasspy_shape() {
    let intent = Intent {
        name: "turn.on".to_string(),
        entities: vec![Entity {
            name: "light".to_string(),
            value: Some(json!("desk lamp")),
        }],
        text: None,
        context: None,
    };
    assert_eq!(
        intent.to_rhasspy(),
        json!({
            "intent": {"name": "turn.on"},
            "entities": [{"entity": "light", "value": "desk lamp"}],
            "slots": {"light": "desk lamp"},
        })
    );
}

#[test]
fn not_recognized_and_intents_stream() {
    roundtrip(&NotRecognized {
        text: Some("sorry".to_string()),
        context: None,
    });
    roundtrip(&IntentsStart { context: None });
    roundtrip(&IntentsStop {
        context: Some(Map::from_iter([("k".to_string(), json!("v"))])),
    });
}

// ---------------------------------------------------------------- handle

#[test]
fn handle_events() {
    roundtrip(&Handled {
        text: Some("done".to_string()),
        context: None,
    });
    roundtrip(&NotHandled {
        text: None,
        context: None,
    });
    roundtrip(&HandledStart { context: None });
    roundtrip(&HandledChunk {
        text: "o".to_string(),
    });
    roundtrip(&HandledStop);
    assert!(HandledStop.to_event().data.is_empty());
}

// ---------------------------------------------------------------- pipeline

#[test]
fn run_pipeline_wire_order_and_defaults() {
    let run = RunPipeline {
        start_stage: PipelineStage::Wake,
        end_stage: PipelineStage::Tts,
        restart_on_end: false,
        wake_word_name: None,
        wake_word_names: None,
        announce_text: None,
    };
    assert_eq!(
        data_json(&run.to_event()),
        r#"{"start_stage":"wake","end_stage":"tts","restart_on_end":false}"#
    );
    roundtrip(&run);
    frame_roundtrip(&run);
}

#[test]
fn run_pipeline_empty_wake_word_names_omitted() {
    let run = RunPipeline {
        start_stage: PipelineStage::Asr,
        end_stage: PipelineStage::Intent,
        restart_on_end: true,
        wake_word_name: Some("hey mycroft".to_string()),
        wake_word_names: Some(Vec::new()),
        announce_text: Some("hello".to_string()),
    };
    assert_eq!(
        data_json(&run.to_event()),
        r#"{"start_stage":"asr","end_stage":"intent","restart_on_end":true,"wake_word_name":"hey mycroft","announce_text":"hello"}"#
    );

    let with_names = RunPipeline {
        wake_word_names: Some(vec!["hey mycroft".to_string()]),
        ..run
    };
    assert_eq!(
        data_json(&with_names.to_event()),
        r#"{"start_stage":"asr","end_stage":"intent","restart_on_end":true,"wake_word_name":"hey mycroft","wake_word_names":["hey mycroft"],"announce_text":"hello"}"#
    );
    roundtrip(&with_names);
}

#[test]
fn run_pipeline_rejects_invalid_stage_combinations() {
    // Upstream __post_init__ table: asr -> wake is invalid.
    let bad = Event {
        type_: "run-pipeline".to_string(),
        data: Map::from_iter([
            ("start_stage".to_string(), json!("asr")),
            ("end_stage".to_string(), json!("wake")),
        ]),
        payload: None,
    };
    assert!(RunPipeline::from_event(&bad).is_err());

    // Valid table entries all decode.
    for (start, ends) in [
        (PipelineStage::Wake, vec![PipelineStage::Wake, PipelineStage::Asr, PipelineStage::Intent, PipelineStage::Handle, PipelineStage::Tts]),
        (PipelineStage::Asr, vec![PipelineStage::Asr, PipelineStage::Intent, PipelineStage::Handle, PipelineStage::Tts]),
        (PipelineStage::Intent, vec![PipelineStage::Intent, PipelineStage::Handle, PipelineStage::Tts]),
        (PipelineStage::Handle, vec![PipelineStage::Handle, PipelineStage::Tts]),
        (PipelineStage::Tts, vec![PipelineStage::Tts]),
    ] {
        for end in ends {
            let run = RunPipeline {
                start_stage: start,
                end_stage: end,
                restart_on_end: false,
                wake_word_name: None,
                wake_word_names: None,
                announce_text: None,
            };
            assert!(run.validate().is_ok());
            roundtrip(&run);
        }
    }

    // Unknown stage string is an error.
    let bad_stage = Event {
        type_: "run-pipeline".to_string(),
        data: Map::from_iter([
            ("start_stage".to_string(), json!("asr")),
            ("end_stage".to_string(), json!("bogus")),
        ]),
        payload: None,
    };
    assert!(RunPipeline::from_event(&bad_stage).is_err());
}

// ---------------------------------------------------------------- satellite

#[test]
fn satellite_events_are_empty() {
    macro_rules! check_empty {
        ($ty:ty, $wire:expr) => {{
            let e = <$ty>::default().to_event();
            assert_eq!(e.type_, $wire);
            assert!(e.data.is_empty());
            assert!(e.payload.is_none());
            <$ty>::from_event(&e).expect("decodes back");
        }};
    }
    check_empty!(RunSatellite, "run-satellite");
    check_empty!(PauseSatellite, "pause-satellite");
    check_empty!(StreamingStarted, "streaming-started");
    check_empty!(StreamingStopped, "streaming-stopped");
    check_empty!(SatelliteConnected, "satellite-connected");
    check_empty!(SatelliteDisconnected, "satellite-disconnected");
}

// ---------------------------------------------------------------- info

fn attribution() -> Attribution {
    Attribution {
        name: "Rhasspy".to_string(),
        url: "https://github.com/rhasspy".to_string(),
    }
}

#[test]
fn describe_and_select_program() {
    let d = Describe.to_event();
    assert_eq!(d.type_, "describe");
    assert!(d.data.is_empty());
    Describe::from_event(&d).unwrap();

    roundtrip(&SelectProgram {
        name: "wyoming-whisper".to_string(),
    });
}

#[test]
fn info_roundtrip_with_defaults_and_order() {
    let info = Info {
        asr: vec![AsrProgram {
            name: "wyoming-whisper".to_string(),
            attribution: attribution(),
            installed: true,
            description: None,
            version: None,
            models: vec![AsrModel {
                name: "small".to_string(),
                attribution: attribution(),
                installed: true,
                description: Some("Small model".to_string()),
                version: None,
                languages: vec!["en".to_string()],
            }],
            supports_transcript_streaming: false,
            requires_external_vad: true,
            prefers_auto_gain_enabled: true,
            prefers_noise_reduction_enabled: true,
        }],
        ..Default::default()
    };
    let event = info.to_event();
    assert_eq!(event.type_, "info");
    // Key order matches upstream to_dict; satellite omitted when None.
    let keys: Vec<&str> = event.data.keys().map(String::as_str).collect();
    assert_eq!(keys, ["asr", "tts", "handle", "intent", "wake", "mic", "snd"]);
    // Artifact base field order.
    let asr0 = &event.data["asr"][0];
    let asr_keys: Vec<&str> = asr0
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        asr_keys,
        [
            "name",
            "attribution",
            "installed",
            "description",
            "version",
            "models",
            "supports_transcript_streaming",
            "requires_external_vad",
            "prefers_auto_gain_enabled",
            "prefers_noise_reduction_enabled"
        ]
    );
    roundtrip(&info);
    frame_roundtrip(&info);
}

#[test]
fn info_deserialization_applies_python_defaults() {
    // A wire info object that omits the optional booleans must pick up the
    // upstream dataclass defaults: streaming=false, vad/gain/noise=true.
    let raw = json!({
        "asr": [{
            "name": "asr1",
            "attribution": {"name": "N", "url": "U"},
            "installed": true,
            "description": null,
            "version": null,
            "models": []
        }],
        "tts": [],
        "handle": [],
        "intent": [],
        "wake": [],
        "mic": [],
        "snd": []
    });
    let event = Event {
        type_: "info".to_string(),
        data: raw.as_object().unwrap().clone(),
        payload: None,
    };
    let info = Info::from_event(&event).unwrap();
    let asr = &info.asr[0];
    assert!(!asr.supports_transcript_streaming);
    assert!(asr.requires_external_vad);
    assert!(asr.prefers_auto_gain_enabled);
    assert!(asr.prefers_noise_reduction_enabled);
}

#[test]
fn info_with_satellite() {
    let info = Info {
        satellite: Some(Satellite {
            name: "satellite".to_string(),
            attribution: attribution(),
            installed: true,
            description: None,
            version: None,
            area: Some("office".to_string()),
            has_vad: Some(true),
            active_wake_words: Some(vec!["hey mycroft".to_string()]),
            max_active_wake_words: Some(2),
            supports_trigger: Some(true),
        }),
        mic: vec![MicProgram {
            name: "mic".to_string(),
            attribution: attribution(),
            installed: true,
            description: None,
            version: None,
            mic_format: AudioFormat {
                rate: 16000,
                width: 2,
                channels: 1,
            },
        }],
        snd: vec![SndProgram {
            name: "snd".to_string(),
            attribution: attribution(),
            installed: true,
            description: None,
            version: None,
            snd_format: AudioFormat {
                rate: 22050,
                width: 2,
                channels: 1,
            },
        }],
        ..Default::default()
    };
    let event = info.to_event();
    let keys: Vec<&str> = event.data.keys().map(String::as_str).collect();
    assert_eq!(keys, ["asr", "tts", "handle", "intent", "wake", "mic", "snd", "satellite"]);
    // Satellite optional fields are always present (may be null).
    let sat = event.data["satellite"].as_object().unwrap();
    let sat_keys: Vec<&str> = sat.keys().map(String::as_str).collect();
    assert_eq!(
        sat_keys,
        [
            "name",
            "attribution",
            "installed",
            "description",
            "version",
            "area",
            "has_vad",
            "active_wake_words",
            "max_active_wake_words",
            "supports_trigger"
        ]
    );
    roundtrip(&info);
}

#[test]
fn tts_voice_with_speakers() {
    let voice = TtsVoice {
        name: "en_US-lessac-medium".to_string(),
        attribution: attribution(),
        installed: true,
        description: None,
        version: None,
        languages: vec!["en".to_string()],
        speakers: Some(vec![TtsVoiceSpeaker {
            name: "lessac".to_string(),
        }]),
    };
    let info = Info {
        tts: vec![TtsProgram {
            name: "wyoming-piper".to_string(),
            attribution: attribution(),
            installed: true,
            description: None,
            version: None,
            voices: vec![voice.clone()],
            supports_synthesize_streaming: true,
        }],
        ..Default::default()
    };
    let event = info.to_event();
    assert_eq!(
        serde_json::to_string(&event.data["tts"][0]["voices"][0]).unwrap(),
        r#"{"name":"en_US-lessac-medium","attribution":{"name":"Rhasspy","url":"https://github.com/rhasspy"},"installed":true,"description":null,"version":null,"languages":["en"],"speakers":[{"name":"lessac"}]}"#
    );
    roundtrip(&info);
}

// ---------------------------------------------------------------- EventKind

#[test]
fn eventkind_dispatch_and_unknown() {
    let cases: Vec<(Event, &str)> = vec![
        (Ping { text: None }.to_event(), "ping"),
        (VoiceStopped { timestamp: None }.to_event(), "voice-stopped"),
        (SynthesizeChunk { text: "x".into() }.to_event(), "synthesize-chunk"),
    ];
    for (event, want_type) in cases {
        let kind = EventKind::from_event(&event).expect("known type");
        assert_eq!(kind.to_event(), event);
        assert_eq!(event.type_, want_type);
    }

    let unknown = Event::new("not-a-real-event");
    match EventKind::from_event(&unknown) {
        Err(EventKindError::UnknownType(t)) => assert_eq!(t, "not-a-real-event"),
        other => panic!("expected UnknownType, got {other:?}"),
    }

    // Known type, wrong data shape.
    let bad = Event {
        type_: "synthesize-chunk".to_string(),
        data: Map::new(),
        payload: None,
    };
    match EventKind::from_event(&bad) {
        Err(EventKindError::Invalid(_)) => {}
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn eventkind_full_stream_roundtrip() {
    // A realistic satellite session: encode a stream of typed events,
    // decode them back one by one and dispatch through EventKind.
    let stream: Vec<EventKind> = vec![
        EventKind::AudioStart(AudioStart {
            rate: 16000,
            width: 2,
            channels: 1,
            timestamp: None,
        }),
        EventKind::AudioChunk(AudioChunk {
            rate: 16000,
            width: 2,
            channels: 1,
            audio: vec![0u8; 1024],
            timestamp: Some(1),
        }),
        EventKind::AudioStop(AudioStop { timestamp: Some(2) }),
        EventKind::Transcript(Transcript {
            text: "hello".to_string(),
            language: None,
            context: None,
        }),
        EventKind::RunPipeline(RunPipeline {
            start_stage: PipelineStage::Wake,
            end_stage: PipelineStage::Tts,
            restart_on_end: true,
            wake_word_name: None,
            wake_word_names: None,
            announce_text: None,
        }),
    ];

    let mut bytes = Vec::new();
    for kind in &stream {
        bytes.extend_from_slice(&encode(&kind.to_event()));
    }
    let mut cursor = Cursor::new(bytes);
    let mut decoded = VecDeque::new();
    while let Some(event) = decode(&mut cursor).unwrap() {
        decoded.push_back(EventKind::from_event(&event).expect("known type"));
    }
    assert_eq!(stream.len(), decoded.len());
    for (want, got) in stream.iter().zip(decoded.iter()) {
        assert_eq!(want, got);
    }
}
