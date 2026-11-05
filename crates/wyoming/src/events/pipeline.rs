//! Pipeline events (upstream `wyoming/pipeline.py`).

use serde_json::{Map, Value};

use crate::error::Error;
use crate::event::Event;
use crate::events::{is_none_or_empty, Eventable};

/// Stages of a pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PipelineStage {
    /// Wake word detection.
    Wake,
    /// Speech-to-text.
    Asr,
    /// Intent recognition.
    Intent,
    /// Intent handling.
    Handle,
    /// Text-to-speech.
    Tts,
}

impl PipelineStage {
    /// Wire string value.
    pub fn as_str(&self) -> &'static str {
        match self {
            PipelineStage::Wake => "wake",
            PipelineStage::Asr => "asr",
            PipelineStage::Intent => "intent",
            PipelineStage::Handle => "handle",
            PipelineStage::Tts => "tts",
        }
    }

    fn from_str(s: &str) -> Result<PipelineStage, Error> {
        match s {
            "wake" => Ok(PipelineStage::Wake),
            "asr" => Ok(PipelineStage::Asr),
            "intent" => Ok(PipelineStage::Intent),
            "handle" => Ok(PipelineStage::Handle),
            "tts" => Ok(PipelineStage::Tts),
            other => Err(Error::InvalidField(format!("invalid pipeline stage: {other}"))),
        }
    }

    /// Valid end stages for this start stage (upstream `__post_init__` table).
    fn valid_ends(start: PipelineStage) -> &'static [PipelineStage] {
        use PipelineStage::*;
        match start {
            Wake => &[Wake, Asr, Intent, Handle, Tts],
            Asr => &[Asr, Intent, Handle, Tts],
            Intent => &[Intent, Handle, Tts],
            Handle => &[Handle, Tts],
            Tts => &[Tts],
        }
    }
}

/// Run a pipeline (`run-pipeline`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RunPipeline {
    /// Stage to start the pipeline on.
    pub start_stage: PipelineStage,
    /// Stage to end the pipeline on.
    pub end_stage: PipelineStage,
    /// True if pipeline should restart automatically after ending.
    #[serde(default)]
    pub restart_on_end: bool,
    /// Name of wake word that triggered this pipeline (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wake_word_name: Option<String>,
    /// Wake word names to listen for (omitted when `None` or empty, like
    /// upstream `if self.wake_word_names:`).
    #[serde(default, skip_serializing_if = "is_none_or_empty")]
    pub wake_word_names: Option<Vec<String>>,
    /// Text to announce using text-to-speech (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub announce_text: Option<String>,
}

impl RunPipeline {
    /// Mirror of upstream `__post_init__` stage-combination validation.
    pub fn validate(&self) -> Result<(), Error> {
        if !Self::valid_end_stages(self.start_stage).contains(&self.end_stage) {
            return Err(Error::InvalidField(format!(
                "invalid end stage: {}",
                self.end_stage.as_str()
            )));
        }
        Ok(())
    }

    /// Valid end stages for a start stage.
    pub fn valid_end_stages(start: PipelineStage) -> &'static [PipelineStage] {
        PipelineStage::valid_ends(start)
    }
}

impl Eventable for RunPipeline {
    const TYPE: &'static str = "run-pipeline";

    fn to_event(&self) -> Event {
        let mut data = Map::new();
        data.insert(
            "start_stage".to_string(),
            Value::String(self.start_stage.as_str().to_string()),
        );
        data.insert(
            "end_stage".to_string(),
            Value::String(self.end_stage.as_str().to_string()),
        );
        data.insert("restart_on_end".to_string(), Value::Bool(self.restart_on_end));
        if let Some(name) = &self.wake_word_name {
            data.insert("wake_word_name".to_string(), Value::String(name.clone()));
        }
        if let Some(names) = &self.wake_word_names {
            if !names.is_empty() {
                data.insert(
                    "wake_word_names".to_string(),
                    serde_json::to_value(names).unwrap_or(Value::Null),
                );
            }
        }
        if let Some(text) = &self.announce_text {
            data.insert("announce_text".to_string(), Value::String(text.clone()));
        }
        Event {
            type_: Self::TYPE.to_string(),
            data,
            payload: None,
        }
    }

    fn from_event(event: &Event) -> Result<Self, Error> {
        if event.type_ != Self::TYPE {
            return Err(Error::InvalidField(format!(
                "expected event type \"{}\", got \"{}\"",
                Self::TYPE,
                event.type_
            )));
        }
        let start_str = event
            .data
            .get("start_stage")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::MissingField("start_stage".to_string()))?;
        let end_str = event
            .data
            .get("end_stage")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::MissingField("end_stage".to_string()))?;
        let run = RunPipeline {
            start_stage: PipelineStage::from_str(start_str)?,
            end_stage: PipelineStage::from_str(end_str)?,
            restart_on_end: event
                .data
                .get("restart_on_end")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            wake_word_name: event
                .data
                .get("wake_word_name")
                .and_then(Value::as_str)
                .map(String::from),
            wake_word_names: event
                .data
                .get("wake_word_names")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                }),
            announce_text: event
                .data
                .get("announce_text")
                .and_then(Value::as_str)
                .map(String::from),
        };
        // Upstream validates in __post_init__ on every construction path,
        // including from_event.
        run.validate()?;
        Ok(run)
    }
}
