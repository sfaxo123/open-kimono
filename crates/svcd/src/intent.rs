//! Zero-LLM voice intent matching (SERVICE-IR.md §3.1, ha-intents sentence
//! syntax, spec-only): `[optional|alt]`, `(required|alt)`, `{slot}` and
//! `{list:slot}` (the slot name is the part after the colon).
//!
//! Matching is on a normalised utterance (lowercase, punctuation dropped,
//! single spaces). Template whitespace matches zero or more spaces, so
//! `hour[s] [and] {minutes}` matches with or without "and".

use std::collections::BTreeMap;

use serde_json::Value;

use crate::ir::SlotType;

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Text(String),
    Space,
    Slot(String),
    /// Alternatives; `optional` adds the empty alternative.
    Group { alts: Vec<Vec<Node>>, optional: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sentence {
    pub nodes: Vec<Node>,
}

impl Sentence {
    pub fn parse(src: &str) -> Result<Self, String> {
        let chars: Vec<char> = src.to_lowercase().chars().collect();
        let mut pos = 0;
        let alts = parse_seq(&chars, &mut pos, None)?;
        if pos != chars.len() {
            return Err(format!("unbalanced `{}` at offset {pos} in `{src}`", chars[pos]));
        }
        if alts.len() != 1 {
            return Err(format!("top-level `|` outside a group in `{src}`"));
        }
        Ok(Sentence { nodes: alts.into_iter().next().unwrap() })
    }

    pub fn slots(&self) -> Vec<String> {
        fn walk(nodes: &[Node], out: &mut Vec<String>) {
            for n in nodes {
                match n {
                    Node::Slot(s) if !out.contains(s) => out.push(s.clone()),
                    Node::Group { alts, .. } => alts.iter().for_each(|a| walk(a, out)),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.nodes, &mut out);
        out
    }

    /// Match a whole utterance. Returns the typed slot values on success.
    pub fn matches(&self, utterance: &str, slot_types: &BTreeMap<String, SlotType>) -> Option<BTreeMap<String, Value>> {
        let text: Vec<char> = normalize(utterance).chars().collect();
        let mut slots = BTreeMap::new();
        let mut k = |end: usize, slots: &BTreeMap<String, Value>| (end == text.len()).then(|| slots.clone());
        match_nodes(&self.nodes, &text, 0, &mut slots, slot_types, &mut k)
    }
}

/// Lowercase, drop punctuation except digits/letters/spaces/'.', collapse spaces.
pub fn normalize(s: &str) -> String {
    let cleaned: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' || c == '.' { c } else { ' ' })
        .collect();
    let cleaned = cleaned.trim_end_matches('.');
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_seq(chars: &[char], pos: &mut usize, close: Option<char>) -> Result<Vec<Vec<Node>>, String> {
    let mut alts = vec![Vec::new()];
    let mut text = String::new();
    let flush = |text: &mut String, alt: &mut Vec<Node>| {
        if !text.is_empty() {
            alt.push(Node::Text(std::mem::take(text)));
        }
    };
    while *pos < chars.len() {
        let c = chars[*pos];
        let alt = alts.last_mut().unwrap();
        match c {
            ']' | ')' => {
                if Some(c) != close {
                    return Err(format!("unexpected `{c}`"));
                }
                flush(&mut text, alt);
                return Ok(alts);
            }
            '[' | '(' => {
                flush(&mut text, alt);
                *pos += 1;
                let inner = parse_seq(chars, pos, Some(if c == '[' { ']' } else { ')' }))?;
                if *pos >= chars.len() {
                    return Err(format!("unclosed `{c}`"));
                }
                alts.last_mut().unwrap().push(Node::Group { alts: inner, optional: c == '[' });
            }
            '{' => {
                flush(&mut text, alt);
                let end = chars[*pos..].iter().position(|&x| x == '}').ok_or("unclosed `{`")? + *pos;
                let body: String = chars[*pos + 1..end].iter().collect();
                let name = body.rsplit(':').next().unwrap_or("").trim().to_string();
                if name.is_empty() || !name.chars().all(|x| x.is_ascii_alphanumeric() || x == '_') {
                    return Err(format!("bad slot `{{{body}}}`"));
                }
                alt.push(Node::Slot(name));
                *pos = end;
            }
            '|' => {
                if close.is_none() {
                    return Err("top-level `|`".into());
                }
                flush(&mut text, alt);
                alts.push(Vec::new());
            }
            '}' => return Err("unexpected `}`".into()),
            c if c.is_whitespace() => {
                flush(&mut text, alt);
                if !matches!(alt.last(), Some(Node::Space)) {
                    alt.push(Node::Space);
                }
            }
            c => text.push(c),
        }
        *pos += 1;
    }
    if close.is_some() {
        return Err("unclosed group".into());
    }
    let alt = alts.last_mut().unwrap();
    flush(&mut text, alt);
    Ok(alts)
}

type Cont<'a> = dyn FnMut(usize, &BTreeMap<String, Value>) -> Option<BTreeMap<String, Value>> + 'a;

/// Continuation-passing backtracking matcher over `text[pos..]`.
fn match_nodes(
    nodes: &[Node],
    text: &[char],
    pos: usize,
    slots: &mut BTreeMap<String, Value>,
    types: &BTreeMap<String, SlotType>,
    k: &mut Cont<'_>,
) -> Option<BTreeMap<String, Value>> {
    let Some((first, rest)) = nodes.split_first() else { return k(pos, slots) };
    match first {
        Node::Text(t) => {
            let t: Vec<char> = t.chars().collect();
            if text.len() >= pos + t.len() && text[pos..pos + t.len()] == t[..] {
                match_nodes(rest, text, pos + t.len(), slots, types, k)
            } else {
                None
            }
        }
        Node::Space => {
            let mut p = pos;
            while p < text.len() && text[p] == ' ' {
                p += 1;
            }
            match_nodes(rest, text, p, slots, types, k)
        }
        Node::Slot(name) => {
            if text.get(pos) == Some(&' ') {
                return None;
            }
            // Slots end at a word boundary (or before `minute[s]`-style suffix
            // text); trying only those keeps matching linear per slot.
            for end in (pos + 1..=text.len()).rev() {
                let boundary = end == text.len() || text[end] == ' ' || !text[end - 1].is_alphanumeric() || text[end - 1].is_ascii_digit() != text[end].is_ascii_digit();
                if !boundary || text[end - 1] == ' ' {
                    continue;
                }
                let raw: String = text[pos..end].iter().collect();
                let Some(value) = slot_value(&raw, types.get(name).copied().unwrap_or(SlotType::String)) else {
                    continue;
                };
                let prev = slots.insert(name.clone(), value);
                if let Some(found) = match_nodes(rest, text, end, slots, types, k) {
                    return Some(found);
                }
                match prev {
                    Some(p) => slots.insert(name.clone(), p),
                    None => slots.remove(name),
                };
            }
            None
        }
        Node::Group { alts, optional } => {
            let empty = Vec::new();
            let options = alts.iter().chain(optional.then_some(&empty));
            for alt in options {
                let mut cont = |p: usize, s: &BTreeMap<String, Value>| {
                    let mut s2 = s.clone();
                    match_nodes(rest, text, p, &mut s2, types, k)
                };
                if let Some(found) = match_nodes(alt, text, pos, slots, types, &mut cont) {
                    return Some(found);
                }
            }
            None
        }
    }
}

const NUMBER_WORDS: [&str; 21] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve",
    "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen", "twenty",
];

fn slot_value(raw: &str, ty: SlotType) -> Option<Value> {
    match ty {
        SlotType::Integer | SlotType::Duration => {
            if raw.chars().all(|c| c.is_ascii_digit()) {
                return raw.parse::<i64>().ok().map(Value::from);
            }
            let tens = [("thirty", 30), ("forty", 40), ("fifty", 50), ("sixty", 60), ("ninety", 90)];
            NUMBER_WORDS
                .iter()
                .position(|w| *w == raw)
                .map(|i| Value::from(i as i64))
                .or_else(|| tens.iter().find(|(w, _)| *w == raw).map(|(_, n)| Value::from(*n)))
        }
        SlotType::Float => raw.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number),
        SlotType::String | SlotType::Enum => Some(Value::String(raw.to_string())),
    }
}
