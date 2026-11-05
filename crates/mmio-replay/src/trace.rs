//! Trace events and the JSONL codec.
//!
//! One flat JSON object per line — the format the oracle recorder on the
//! phone emits and every host test consumes:
//!
//! ```text
//! {"op":"w","width":32,"addr":"0xa84010","value":"0x2","delay_us":0}
//! ```
//!
//! Keys: `op` (`"r"`/`"w"`, also `"read"`/`"write"`), `width` (8/16/32/64),
//! `addr`, `value`, `delay_us` (optional, time since the previous event).
//! `addr`/`value` are written as hex strings but decimal numbers are also
//! accepted when parsing. Unknown keys are ignored so oracle captures can
//! carry extra fields; missing required keys or malformed JSON are errors
//! with the offending 1-based line number.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::mmio::{Op, Width};

/// One recorded MMIO transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Event {
    /// Read or write.
    pub op: Op,
    /// Access width.
    pub width: Width,
    /// Physical address of the access.
    pub addr: u64,
    /// For a write: the value written. For a read: the value the hardware
    /// returned (the replayer feeds this back to the driver).
    pub value: u64,
    /// Microseconds between the previous event and this one (0 if unknown).
    pub delay_us: u64,
}

impl Event {
    /// A read event.
    pub fn read(addr: u64, width: Width, value: u64) -> Self {
        Self {
            op: Op::Read,
            width,
            addr,
            value,
            delay_us: 0,
        }
    }

    /// A write event.
    pub fn write(addr: u64, width: Width, value: u64) -> Self {
        Self {
            op: Op::Write,
            width,
            addr,
            value,
            delay_us: 0,
        }
    }

    /// Builder-style setter for [`Event::delay_us`].
    pub fn with_delay_us(mut self, delay_us: u64) -> Self {
        self.delay_us = delay_us;
        self
    }

    /// Render as one JSONL line (no trailing newline).
    pub fn to_json_line(&self) -> String {
        format!(
            "{{\"op\":\"{}\",\"width\":{},\"addr\":\"{:#x}\",\"value\":\"{:#x}\",\"delay_us\":{}}}",
            if self.op == Op::Read { "r" } else { "w" },
            self.width.bits(),
            self.addr,
            self.value & self.width.mask(),
            self.delay_us,
        )
    }

    /// Parse one JSONL line into an [`Event`].
    pub fn from_json_line(line: &str) -> Result<Event, String> {
        let mut p = Parser::new(line);
        p.parse_event()
    }
}

/// A register trace: MMIO transactions in the order they happened.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Trace {
    /// Events in occurrence order.
    pub events: Vec<Event>,
}

impl Trace {
    /// An empty trace.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an event.
    pub fn push(&mut self, event: Event) {
        self.events.push(event);
    }

    /// Number of recorded transactions.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the trace is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Sum of all recorded inter-event delays, for power/timing budget tests.
    pub fn total_delay_us(&self) -> u64 {
        self.events.iter().map(|e| e.delay_us).sum()
    }

    /// Render as JSONL: one line per event, newline-terminated.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for e in &self.events {
            out.push_str(&e.to_json_line());
            out.push('\n');
        }
        out
    }

    /// Parse JSONL (blank lines skipped). Errors carry the 1-based line.
    pub fn from_jsonl(text: &str) -> Result<Trace, ParseError> {
        let mut trace = Trace::new();
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let event = Event::from_json_line(line).map_err(|m| ParseError {
                line: i + 1,
                message: m,
            })?;
            trace.events.push(event);
        }
        Ok(trace)
    }
}

/// A JSONL parse failure with the 1-based line number.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParseError {
    /// 1-based line number of the offending line.
    pub line: usize,
    /// What went wrong.
    pub message: String,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

enum Scalar {
    Str(String),
    Num(u64),
}

struct Parser<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            b: s.as_bytes(),
            pos: 0,
        }
    }

    fn err<T>(msg: &str) -> Result<T, String> {
        Err(String::from(msg))
    }

    fn skip_ws(&mut self) {
        while self.pos < self.b.len() && (self.b[self.pos] as char).is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.b.get(self.pos).copied()
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.peek() != Some(c) {
            Self::err(&format!(
                "expected '{}', found {:?}",
                c as char,
                self.peek()
            ))
        } else {
            self.pos += 1;
            Ok(())
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let c = match self.b.get(self.pos) {
                Some(c) => *c,
                None => return Self::err("unterminated string"),
            };
            self.pos += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = match self.b.get(self.pos) {
                        Some(e) => *e,
                        None => return Self::err("unterminated escape"),
                    };
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        _ => return Self::err("unsupported string escape"),
                    }
                }
                _ => out.push(c as char),
            }
        }
    }

    fn parse_scalar(&mut self) -> Result<Scalar, String> {
        match self.peek() {
            Some(b'"') => Ok(Scalar::Str(self.parse_string()?)),
            Some(c) if c.is_ascii_digit() => {
                let start = self.pos;
                while self.pos < self.b.len() && self.b[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                let text =
                    core::str::from_utf8(&self.b[start..self.pos]).map_err(|_| "non-utf8")?;
                text.parse::<u64>()
                    .map(Scalar::Num)
                    .map_err(|_| format!("number out of range: {text}"))
            }
            _ => Self::err("expected a string or number value"),
        }
    }

    fn parse_uint(&mut self) -> Result<u64, String> {
        match self.parse_scalar()? {
            Scalar::Num(n) => Ok(n),
            Scalar::Str(s) => parse_u64_text(&s),
        }
    }

    fn parse_event(&mut self) -> Result<Event, String> {
        self.expect(b'{')?;
        let mut op: Option<Op> = None;
        let mut width: Option<Width> = None;
        let mut addr: Option<u64> = None;
        let mut value: Option<u64> = None;
        let mut delay_us: Option<u64> = None;

        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Self::err("empty object");
        }

        loop {
            let key = self.parse_string()?;
            self.expect(b':')?;
            match key.as_str() {
                "op" => {
                    let s = match self.parse_scalar()? {
                        Scalar::Str(s) => s,
                        Scalar::Num(_) => return Self::err("op must be \"r\" or \"w\""),
                    };
                    op = Some(match s.as_str() {
                        "r" | "read" => Op::Read,
                        "w" | "write" => Op::Write,
                        other => return Self::err(&format!("unknown op {other:?}")),
                    });
                }
                "width" => {
                    let bits = self.parse_uint()?;
                    width = Some(match Width::from_bits(bits as u32) {
                        Some(w) if bits <= 64 => w,
                        _ => return Self::err("width must be 8, 16, 32 or 64"),
                    });
                }
                "addr" => addr = Some(self.parse_uint()?),
                "value" => value = Some(self.parse_uint()?),
                "delay_us" => delay_us = Some(self.parse_uint()?),
                _ => {
                    // Unknown keys are tolerated (oracle captures may carry
                    // extra fields) but the value must still be valid JSON.
                    self.parse_scalar()?;
                }
            }
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Self::err("expected ',' or '}'"),
            }
        }
        // Trailing content after the object is a parse error.
        if self.peek().is_some() {
            return Self::err("trailing characters after object");
        }

        let event = Event {
            op: op.ok_or("missing key \"op\"")?,
            width: width.ok_or("missing key \"width\"")?,
            addr: addr.ok_or("missing key \"addr\"")?,
            value: value.ok_or("missing key \"value\"")?,
            delay_us: delay_us.unwrap_or(0),
        };
        Ok(event)
    }
}

/// Parse a decimal or `0x`-hex unsigned integer from text.
fn parse_u64_text(s: &str) -> Result<u64, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err(String::from("empty number"));
    }
    let (digits, radix) = if let Some(rest) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X"))
    {
        (rest, 16)
    } else {
        (t, 10)
    };
    if digits.is_empty() {
        return Err(format!("no digits in {s:?}"));
    }
    u64::from_str_radix(digits, radix).map_err(|_| format!("invalid number {s:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev() -> Event {
        Event::write(0xa84010, Width::W32, 2).with_delay_us(12)
    }

    #[test]
    fn line_roundtrip() {
        let e = ev();
        let line = e.to_json_line();
        assert_eq!(
            line,
            r#"{"op":"w","width":32,"addr":"0xa84010","value":"0x2","delay_us":12}"#
        );
        assert_eq!(Event::from_json_line(&line).unwrap(), e);
    }

    #[test]
    fn trace_roundtrip_with_blank_lines() {
        let mut t = Trace::new();
        t.push(ev());
        t.push(Event::read(0xa84018, Width::W32, 0x4000_0000).with_delay_us(3));
        let jsonl = t.to_jsonl();
        let parsed = Trace::from_jsonl(&format!("\n{jsonl}\n")).unwrap();
        assert_eq!(parsed, t);
        assert_eq!(parsed.total_delay_us(), 15);
    }

    #[test]
    fn accepts_decimal_and_long_op_spellings() {
        let e = Event::from_json_line(
            r#"{"op":"read","width":16,"addr":43087,"value":258,"delay_us":1}"#,
        )
        .unwrap();
        assert_eq!(e, Event::read(0xa84f, Width::W16, 0x102).with_delay_us(1));
    }

    #[test]
    fn delay_us_is_optional() {
        let e =
            Event::from_json_line(r#"{"op":"w","width":8,"addr":"0x10","value":"0xff"}"#).unwrap();
        assert_eq!(e.delay_us, 0);
    }

    #[test]
    fn unknown_keys_ignored() {
        let e = Event::from_json_line(
            r#"{"op":"w","width":32,"addr":"0x10","value":"0x1","device":"geni9","seq":7}"#,
        )
        .unwrap();
        assert_eq!(e.addr, 0x10);
    }

    #[test]
    fn write_value_masked_on_output() {
        let e = Event::write(0x10, Width::W8, 0x1ff);
        assert!(e.to_json_line().contains("\"value\":\"0xff\""));
    }

    #[test]
    fn parse_errors_name_the_problem() {
        assert_eq!(
            Event::from_json_line(r#"{"width":32,"addr":"0x10","value":"0x1"}"#).unwrap_err(),
            "missing key \"op\""
        );
        assert_eq!(
            Event::from_json_line(r#"{"op":"x","width":32,"addr":"0x10","value":"0x1"}"#)
                .unwrap_err(),
            "unknown op \"x\""
        );
        assert_eq!(
            Event::from_json_line(r#"{"op":"w","width":12,"addr":"0x10","value":"0x1"}"#)
                .unwrap_err(),
            "width must be 8, 16, 32 or 64"
        );
        assert!(Event::from_json_line("{").is_err());
        assert!(Event::from_json_line(
            r#"{"op":"w","width":32,"addr":"0x10","value":"0x1" extra}"#
        )
        .is_err());
        assert!(Event::from_json_line(r#"{"op":"w","width":32,"addr":"zz","value":1}"#).is_err());
        assert!(Event::from_json_line(r#"{"op":"w","width":32,"addr":1.5,"value":1}"#).is_err());
    }

    #[test]
    fn jsonl_errors_carry_line_numbers() {
        let text = "not json\n{\"op\":\"w\",\"width\":32,\"addr\":\"0x10\",\"value\":\"0x1\"}\n{\"op\":\"w\"}\n";
        let err = Trace::from_jsonl(text).unwrap_err();
        assert_eq!(err.line, 1);
        assert_eq!(Trace::from_jsonl("not json\n\n\n").unwrap_err().line, 1);
        // First two lines fine, third bad:
        let text2 = "{\"op\":\"w\",\"width\":32,\"addr\":\"0x10\",\"value\":\"0x1\"}\n\n{\"op\":\"q\",\"width\":32,\"addr\":\"0x10\",\"value\":\"0x1\"}\n";
        assert_eq!(Trace::from_jsonl(text2).unwrap_err().line, 3);
    }

    #[test]
    fn number_overflow_rejected() {
        assert!(Event::from_json_line(
            r#"{"op":"w","width":32,"addr":99999999999999999999,"value":1}"#
        )
        .is_err());
    }
}
