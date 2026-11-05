//! Timer events (upstream `wyoming/timer.py`).

use crate::error::Error;
use crate::event::Event;
use crate::events::{from_data, to_map, Eventable};

/// New timer was started (`timer-started`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TimerStarted {
    /// Unique id of timer.
    pub id: String,
    /// Total number of seconds the timer will run for.
    pub total_seconds: u64,
    /// Optional name provided by user (omitted when `None`, like upstream
    /// `if self.name is not None:`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Number of hours users requested the timer to run for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_hours: Option<u64>,
    /// Number of minutes users requested the timer to run for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_minutes: Option<u64>,
    /// Number of seconds users requested the timer to run for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<u64>,
}

/// Existing timer was paused, resumed, or had time added or removed
/// (`timer-updated`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TimerUpdated {
    /// Unique id of timer.
    pub id: String,
    /// True if timer is running.
    pub is_active: bool,
    /// Number of seconds left on the timer.
    pub total_seconds: u64,
}

/// Existing timer was cancelled (`timer-cancelled`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TimerCancelled {
    /// Unique id of timer.
    pub id: String,
}

/// Existing timer finished without being cancelled (`timer-finished`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TimerFinished {
    /// Unique id of timer.
    pub id: String,
}

macro_rules! impl_timer_eventable {
    ($ty:ident, $wire:literal) => {
        impl Eventable for $ty {
            const TYPE: &'static str = $wire;

            fn to_event(&self) -> Event {
                Event {
                    type_: Self::TYPE.to_string(),
                    data: to_map(self),
                    payload: None,
                }
            }

            fn from_event(event: &Event) -> Result<Self, Error> {
                from_data(event, Self::TYPE)
            }
        }
    };
}

impl_timer_eventable!(TimerStarted, "timer-started");
impl_timer_eventable!(TimerUpdated, "timer-updated");
impl_timer_eventable!(TimerCancelled, "timer-cancelled");
impl_timer_eventable!(TimerFinished, "timer-finished");
