//! Satellite control events (upstream `wyoming/satellite.py`).
//!
//! All satellite events carry no data fields.

use crate::error::Error;
use crate::event::Event;
use crate::events::Eventable;

macro_rules! empty_satellite_event {
    ($name:ident, $wire:literal) => {
        #[doc = concat!("Satellite event (`", $wire, "`), no data fields.")]
        #[derive(Debug, Clone, PartialEq, Eq, Default)]
        pub struct $name;

        impl Eventable for $name {
            const TYPE: &'static str = $wire;

            fn to_event(&self) -> Event {
                Event::new(Self::TYPE)
            }

            fn from_event(event: &Event) -> Result<Self, Error> {
                if event.type_ != Self::TYPE {
                    return Err(Error::InvalidField(format!(
                        "expected event type \"{}\", got \"{}\"",
                        Self::TYPE,
                        event.type_
                    )));
                }
                Ok($name)
            }
        }
    };
}

empty_satellite_event!(RunSatellite, "run-satellite");
empty_satellite_event!(PauseSatellite, "pause-satellite");
empty_satellite_event!(StreamingStarted, "streaming-started");
empty_satellite_event!(StreamingStopped, "streaming-stopped");
empty_satellite_event!(SatelliteConnected, "satellite-connected");
empty_satellite_event!(SatelliteDisconnected, "satellite-disconnected");
