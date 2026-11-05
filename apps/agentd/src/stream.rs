//! Sentence chunking for streamed answers (CV3).
//!
//! The brain's answer arrives as token deltas; TTS wants sentences.
//! The chunker accumulates deltas and emits a sentence only when its
//! end is *proven* — a terminator (`.` / `!` / `?`, plus any closing
//! quotes or brackets) followed by whitespace already in the buffer
//! and a plausible next-sentence start (a capital, a digit, an
//! opening quote): a lowercase continuation after an ellipsis is
//! the same sentence still thinking, not a new one.
//! A terminator at the very end of the buffer proves nothing yet: the
//! next delta may continue the same sentence (the classic case is a
//! decimal split across deltas, "arrive at 3." + "5 pm"), so the tail
//! is held until more text arrives or the stream finishes.
//!
//! Conservative rules, each pinned by a test below:
//! - decimals: a `.` between two digits is not a boundary ("2.5");
//! - abbreviations: a `.` after a known abbreviation ("Dr.", "St.",
//!   "U.S.", "e.g.", "p.m.") or a single capital initial ("A.") is
//!   not a boundary — the cost of a miss is a longer chunk, never a
//!   wrong cut, which is the safe side for speech;
//! - an ellipsis is one terminator, decided at its last dot;
//! - terminator runs ("Really?!") end one sentence, at the run's end;
//! - closing quotes/brackets after the terminator belong to the
//!   sentence they close (`"Go home."` keeps its quote).
//!
//! The maps answer shape ("Maps is open with directions to San
//! Francisco. The drive takes about 2.5 hours via I-80.") exercises
//! the plain path and the decimal rule; street names exercise the
//! abbreviation rule ("Main St." never splits a chunk in two).

/// Accumulates streamed content and emits proven-complete sentences.
pub struct SentenceChunker {
    buf: String,
}

const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "st", "ave", "blvd", "rd", "mt", "ft", "vs", "approx", "appt",
    "dept", "est", "fig", "jan", "feb", "mar", "apr", "jun", "jul", "aug", "sep", "sept", "oct",
    "nov", "dec", "e.g", "i.e", "u.s", "u.s.a", "a.m", "p.m",
];

impl SentenceChunker {
    pub fn new() -> SentenceChunker {
        SentenceChunker { buf: String::new() }
    }

    /// Feed one content delta; returns the sentences it proved complete.
    pub fn push(&mut self, delta: &str) -> Vec<String> {
        self.buf.push_str(delta);
        let mut out = Vec::new();
        while let Some((end, rest)) = find_boundary(&self.buf) {
            let sentence = self.buf[..end].trim().to_string();
            self.buf.drain(..rest);
            if !sentence.is_empty() {
                out.push(sentence);
            }
        }
        out
    }

    /// The stream is over: whatever remains is the final sentence
    /// (or nothing, when the answer ended exactly on a boundary).
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buf);
        let t = rest.trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    }
}

impl Default for SentenceChunker {
    fn default() -> Self {
        Self::new()
    }
}

/// The first proven sentence boundary in `buf`: (sentence end
/// exclusive, remainder start) as byte indices.
fn find_boundary(buf: &str) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < buf.len() {
        let c = buf[i..].chars().next().expect("i is a char boundary");
        let clen = c.len_utf8();
        if matches!(c, '.' | '!' | '?') {
            if c == '.' {
                // Decimal point: a digit on both sides ("2.5").
                let prev_digit =
                    buf[..i].chars().next_back().map(|p| p.is_ascii_digit()).unwrap_or(false);
                let next_digit =
                    buf[i + clen..].chars().next().map(|n| n.is_ascii_digit()).unwrap_or(false);
                if prev_digit && next_digit {
                    i += clen;
                    continue;
                }
                // Ellipsis: only the last dot of the run may decide.
                if buf[i + clen..].starts_with('.') {
                    i += clen;
                    continue;
                }
                // Abbreviation or capital initial: not a boundary.
                let token = token_before(buf, i);
                let lower = token.to_lowercase();
                let initial = token.chars().count() == 1
                    && token.chars().next().map(|t| t.is_ascii_uppercase()).unwrap_or(false);
                if ABBREVIATIONS.contains(&lower.as_str()) || initial {
                    i += clen;
                    continue;
                }
            }
            // Extend over the terminator run and closing quotes/brackets.
            let mut j = i + clen;
            while j < buf.len() {
                let n = buf[j..].chars().next().expect("j is a char boundary");
                if matches!(n, '.' | '!' | '?' | '"' | '\'' | ')' | ']' | '”' | '’') {
                    j += n.len_utf8();
                } else {
                    break;
                }
            }
            // Proven only by following whitespace AND a plausible
            // next-sentence start (a capital, a digit, an opening
            // quote): with nothing after the terminator yet, the
            // next delta could still continue this sentence, so the
            // boundary waits; and a lowercase continuation is the
            // same sentence still thinking ("Wait... really?!"),
            // not a new one.
            if j < buf.len() && buf[j..].chars().next().map(|w| w.is_whitespace()).unwrap_or(false) {
                let mut rest = j;
                while rest < buf.len() {
                    let w = buf[rest..].chars().next().expect("rest is a char boundary");
                    if w.is_whitespace() {
                        rest += w.len_utf8();
                    } else {
                        break;
                    }
                }
                if rest < buf.len() {
                    let first = buf[rest..].chars().next().expect("rest is a char boundary");
                    if first.is_uppercase()
                        || first.is_ascii_digit()
                        || matches!(first, '"' | '\'' | '“' | '‘' | '(')
                    {
                        return Some((j, rest));
                    }
                }
            }
            i += clen;
            continue;
        }
        i += clen;
    }
    None
}

/// The word immediately before byte `end`, with any leading quote or
/// bracket stripped — the token an abbreviation check should see.
fn token_before<'a>(buf: &'a str, end: usize) -> &'a str {
    let s = &buf[..end];
    let token = s.split_whitespace().next_back().unwrap_or("");
    token.trim_start_matches(|c| matches!(c, '"' | '\'' | '(' | '[' | '“' | '‘'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(deltas: &[&str]) -> (Vec<String>, Option<String>) {
        let mut ch = SentenceChunker::new();
        let mut out = Vec::new();
        for d in deltas {
            out.extend(ch.push(d));
        }
        (out, ch.finish())
    }

    #[test]
    fn two_plain_sentences_split_across_deltas() {
        let (done, tail) = run(&["Maps is open with directions to San ", "Francisco. The drive is next."]);
        assert_eq!(done, vec!["Maps is open with directions to San Francisco.".to_string()]);
        assert_eq!(tail.as_deref(), Some("The drive is next."));
    }

    #[test]
    fn decimal_point_is_not_a_boundary_even_split_across_deltas() {
        let (done, tail) = run(&["It takes 2.", "5 hours to get there. Drive safe"]);
        assert_eq!(done, vec!["It takes 2.5 hours to get there.".to_string()]);
        assert_eq!(tail.as_deref(), Some("Drive safe"));
    }

    #[test]
    fn abbreviations_and_initials_do_not_split() {
        // Every internal dot is an abbreviation or an initial, so no
        // boundary is ever proven mid-stream; the text arrives whole
        // as the final sentence at finish(). (An earlier draft of
        // this test expected the push itself to emit the string,
        // which contradicts the buffer-end rule: a terminator at the
        // end of the buffer proves nothing yet. The no-split
        // property this test names is what is asserted.)
        let (done, tail) = run(&["Dr. A. Smith lives on Main St. in the U.S. He left at 5 p.m. Bye."]);
        assert!(done.is_empty(), "no boundary may be proven: {done:?}");
        assert_eq!(
            tail.as_deref(),
            Some("Dr. A. Smith lives on Main St. in the U.S. He left at 5 p.m. Bye.")
        );
    }

    #[test]
    fn ellipsis_and_terminator_runs_end_one_sentence() {
        let (done, tail) = run(&["Wait... really?! Yes."]);
        assert_eq!(done, vec!["Wait... really?!".to_string()]);
        assert_eq!(tail.as_deref(), Some("Yes."));
    }

    #[test]
    fn closing_quote_belongs_to_the_sentence() {
        let (done, tail) = run(&["He said \"Go home.\" Then he left."]);
        assert_eq!(done, vec!["He said \"Go home.\"".to_string()]);
        assert_eq!(tail.as_deref(), Some("Then he left."));
    }

    #[test]
    fn a_terminator_at_the_buffer_end_waits_for_proof() {
        let mut ch = SentenceChunker::new();
        assert!(ch.push("Arrive at 3.").is_empty(), "no proof yet: the next delta may continue");
        assert_eq!(ch.push("5 pm, they said. Done."), vec!["Arrive at 3.5 pm, they said.".to_string()]);
        assert_eq!(ch.finish().as_deref(), Some("Done."));
    }
}

// ---------------------------------------------------------------------
// The `answer` tool's text, recovered while its arguments stream.
//
// agentd's answers usually arrive as a tool call —
// `answer {"text": "..."}` — whose arguments are one JSON object
// built up from fragments. This extractor is a small state machine
// over the concatenated fragments: it finds the `"text"` key, then
// decodes the JSON string value character by character (escapes
// included), robust to a fragment boundary landing anywhere —
// mid-key, mid-escape, mid-\u sequence. What it yields is the
// answer text as the model typed it, early enough to chunk and
// speak before the call's arguments are even complete.
//
// It never invents: before the value's opening quote it yields
// nothing, and once the value's closing quote lands it yields
// nothing more. If the arguments carry no text string, it simply
// never speaks — the caller validates the assembled arguments at
// end-of-stream regardless.

#[derive(Clone, Copy, PartialEq)]
enum ExtractState {
    /// Scanning for the `"text"` key.
    SeekKey,
    /// Key seen; scanning for the value's opening quote.
    SeekValue,
    /// Inside the string value.
    InValue,
    /// Inside a `\` escape in the value.
    Escape,
    /// Inside a `\uXXXX` escape; payload is hex digits remaining.
    Unicode(u8),
    /// The value closed (or never came); nothing more to yield.
    Done,
}

pub struct AnswerTextExtractor {
    state: ExtractState,
    key_matched: usize,
    unicode_acc: u32,
    pending_high: Option<u16>,
}

const KEY: &str = "\"text\"";

impl AnswerTextExtractor {
    pub fn new() -> AnswerTextExtractor {
        AnswerTextExtractor {
            state: ExtractState::SeekKey,
            key_matched: 0,
            unicode_acc: 0,
            pending_high: None,
        }
    }

    /// Feed one arguments fragment; returns the answer text it
    /// newly proves (possibly empty).
    pub fn feed(&mut self, fragment: &str) -> String {
        let mut out = String::new();
        for c in fragment.chars() {
            match self.state {
                ExtractState::SeekKey => {
                    let want = KEY.chars().nth(self.key_matched).unwrap_or('\0');
                    if c == want {
                        self.key_matched += 1;
                        if self.key_matched == KEY.len() {
                            self.state = ExtractState::SeekValue;
                        }
                    } else {
                        self.key_matched = if c == '"' { 1 } else { 0 };
                    }
                }
                ExtractState::SeekValue => {
                    if c == '"' {
                        self.state = ExtractState::InValue;
                    } else if c == '}' {
                        self.state = ExtractState::Done;
                    }
                }
                ExtractState::InValue => match c {
                    '"' => self.state = ExtractState::Done,
                    '\\' => self.state = ExtractState::Escape,
                    _ => self.push_char(&mut out, c),
                },
                ExtractState::Escape => match c {
                    'u' => {
                        self.unicode_acc = 0;
                        self.state = ExtractState::Unicode(4);
                    }
                    _ => {
                        self.state = ExtractState::InValue;
                        match c {
                            'n' => out.push('\n'),
                            't' => out.push('\t'),
                            'r' => out.push('\r'),
                            '"' => out.push('"'),
                            '\\' => out.push('\\'),
                            '/' => out.push('/'),
                            'b' => out.push('\u{0008}'),
                            'f' => out.push('\u{000C}'),
                            other => {
                                out.push('\\');
                                out.push(other);
                            }
                        }
                    }
                },
                ExtractState::Unicode(left) => match c.to_digit(16) {
                    Some(d) => {
                        self.unicode_acc = self.unicode_acc * 16 + d;
                        if left <= 1 {
                            self.state = ExtractState::InValue;
                            self.push_code_unit(&mut out, self.unicode_acc as u16);
                        } else {
                            self.state = ExtractState::Unicode(left - 1);
                        }
                    }
                    None => {
                        // Not hex: the escape was malformed. Yield
                        // the replacement char and carry on as text.
                        self.state = ExtractState::InValue;
                        out.push('\u{FFFD}');
                        self.push_char(&mut out, c);
                    }
                },
                ExtractState::Done => {}
            }
        }
        out
    }

    fn push_char(&mut self, out: &mut String, c: char) {
        if self.pending_high.take().is_some() {
            // A high surrogate whose pair never came.
            out.push('\u{FFFD}');
        }
        out.push(c);
    }

    fn push_code_unit(&mut self, out: &mut String, unit: u16) {
        match unit {
            0xD800..=0xDBFF => {
                if self.pending_high.take().is_some() {
                    out.push('\u{FFFD}');
                }
                self.pending_high = Some(unit);
            }
            0xDC00..=0xDFFF => match self.pending_high.take() {
                Some(high) => {
                    let cp = 0x1_0000u32
                        + (((high as u32 - 0xD800) << 10) | (unit as u32 - 0xDC00));
                    out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                }
                None => out.push('\u{FFFD}'),
            },
            _ => {
                if self.pending_high.take().is_some() {
                    out.push('\u{FFFD}');
                }
                out.push(char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}'));
            }
        }
    }
}

#[cfg(test)]
mod extractor_tests {
    use super::*;

    #[test]
    fn extracts_text_across_fragment_boundaries() {
        let mut ex = AnswerTextExtractor::new();
        assert_eq!(ex.feed("{\"tex"), "");
        assert_eq!(ex.feed("t\": \"Maps is"), "Maps is");
        assert_eq!(ex.feed(" open. Drive"), " open. Drive");
        assert_eq!(ex.feed(" safe.\"}"), " safe.");
        assert_eq!(ex.feed("more"), "");
    }

    #[test]
    fn decodes_escapes_and_unicode() {
        let mut ex = AnswerTextExtractor::new();
        let out = ex.feed("{\"text\": \"She said \\\"go\\\".\\nCaf\\u00e9.\"}");
        assert_eq!(out, "She said \"go\".\nCafé.");
    }

    #[test]
    fn no_text_key_no_output() {
        let mut ex = AnswerTextExtractor::new();
        assert_eq!(ex.feed("{\"destination\": \"San Francisco\"}"), "");
        assert_eq!(ex.feed(""), "");
    }
}
