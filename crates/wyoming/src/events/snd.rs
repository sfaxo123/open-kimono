//! Audio output event (upstream `wyoming/snd.py`).

use crate::error::Error;
use crate::event::Event;
use crate::events::Eventable;

/// Audio has finished playing (`played`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Played;

impl Eventable for Played {
    const TYPE: &'static str = "played";

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
        Ok(Played)
    }
}
