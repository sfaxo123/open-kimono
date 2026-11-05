//! The deterministic expression/template subset and the voice matcher.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use svcd::expr::{self, Scope};
use svcd::intent::Sentence;
use svcd::ir::SlotType;
use svcd::template;

struct Ctx(Value);
impl Scope for Ctx {
    fn root(&self, name: &str) -> Option<Value> {
        self.0.get(name).cloned()
    }
}

fn ctx() -> Ctx {
    Ctx(json!({
        "state": { "armed": true, "discharge_cycle_id": 1, "last_triggered_pct": 19.0, "eventName": "Tea" },
        "params": { "hours": 1, "minutes": 5, "seconds": 3, "name": "Timer" },
        "trigger": { "payload": { "charging": false, "capacity_pct": 19.0 } },
        "utterance_id": "utt-1"
    }))
}

fn eval(src: &str) -> Result<Value, String> {
    expr::parse(src).map_err(|e| e.0)?.eval(&ctx()).map_err(|e| e.0)
}

#[test]
fn precedence_and_integer_arithmetic_stay_integers() {
    assert_eq!(eval("params.hours * 3600 + params.minutes * 60 + params.seconds"), Ok(json!(3903)));
    assert_eq!(eval("(1 + 2) * 3"), Ok(json!(9)));
    assert_eq!(eval("10 - 2 - 3"), Ok(json!(5)), "left associative");
    assert_eq!(eval("7 / 2"), Ok(json!(3.5)), "inexact integer division becomes a float");
    assert_eq!(eval("8 / 2"), Ok(json!(4)));
}

#[test]
fn boolean_guards_from_the_examples() {
    assert_eq!(eval("trigger.payload.charging == false && state.armed == true"), Ok(json!(true)));
    assert_eq!(eval("state.last_triggered_pct < 20"), Ok(json!(true)));
    assert_eq!(eval("state.last_triggered_pct == 19"), Ok(json!(true)), "19.0 == 19");
    assert_eq!(eval("!state.armed || false"), Ok(json!(false)));
    assert_eq!(eval("state.eventName == 'Tea'"), Ok(json!(true)));
}

#[test]
fn strict_errors_instead_of_guesses() {
    assert!(eval("state.missing == 1").unwrap_err().contains("undefined path `state.missing`"));
    assert!(eval("nobody.home").unwrap_err().contains("undefined name"));
    assert!(eval("1 / 0").is_err(), "division by zero");
    assert!(eval("1.0 / 0").unwrap_err().contains("non-finite"), "no NaN/inf ever enters state");
    assert!(eval("'a' < 1").is_err(), "no cross-type comparison");
    assert!(eval("state.armed && 1").is_err(), "no truthiness");
    assert!(eval("9223372036854775807 + 1").unwrap_err().contains("overflow"));
}

#[test]
fn malformed_expressions_are_rejected_at_parse_time() {
    for bad in ["state.armed = true", "(1 + 2", "1 +", "state.", "'open", "a ? b : c", ""] {
        assert!(expr::parse(bad).is_err(), "`{bad}` parsed");
    }
}

#[test]
fn whole_string_template_is_typed_mixed_is_text() {
    let c = ctx();
    let typed = template::parse("{{params.hours * 3600 + params.minutes * 60 + params.seconds}}").unwrap();
    assert_eq!(typed.render(&c).unwrap(), json!(3903));
    let text = template::parse("{{params.name}} set for {{params.minutes}} minutes.").unwrap();
    assert_eq!(text.render(&c).unwrap(), json!("Timer set for 5 minutes."));
    assert_eq!(template::parse("$.trigger.payload.capacity_pct").unwrap().render(&c).unwrap(), json!(19.0));
    assert_eq!(template::parse("plain text").unwrap().render(&c).unwrap(), json!("plain text"));
}

#[test]
fn integral_floats_render_without_a_decimal_point() {
    let s = template::render_text("Battery is down to {{state.last_triggered_pct}} percent.", &ctx()).unwrap();
    assert_eq!(s, "Battery is down to 19 percent.");
    assert_eq!(template::display(&json!(19.5)), "19.5");
}

#[test]
fn render_value_walks_nested_json() {
    let v = template::render_value(&json!({ "a": ["{{params.minutes}}", "$.state.eventName"], "b": 1 }), &ctx()).unwrap();
    assert_eq!(v, json!({ "a": [5, "Tea"], "b": 1 }));
}

#[test]
fn bad_templates_are_rejected() {
    assert!(template::parse("{{params.name").is_err());
    assert!(template::parse("x }} y").is_err());
    assert!(template::parse("$.state..x").is_err());
}

fn slots(pairs: &[(&str, SlotType)]) -> BTreeMap<String, SlotType> {
    pairs.iter().map(|(k, t)| (k.to_string(), *t)).collect()
}

#[test]
fn timer_sentences_match_with_typed_slots() {
    let types = slots(&[("hours", SlotType::Integer), ("minutes", SlotType::Integer), ("seconds", SlotType::Integer), ("name", SlotType::String)]);
    let s = Sentence::parse("timer for {minutes} minute[s]").unwrap();
    assert_eq!(s.matches("timer for 5 minutes", &types).unwrap()["minutes"], json!(5));
    assert_eq!(s.matches("Timer for one minute.", &types).unwrap()["minutes"], json!(1));
    assert!(s.matches("timer for five hours", &types).is_none());
    assert!(s.matches("timer for many minutes", &types).is_none(), "integer slot refuses words it cannot parse");

    let named = Sentence::parse("set a {minutes} minute[s] timer [named {name}]").unwrap();
    let m = named.matches("set a 10 minute timer named green tea", &types).unwrap();
    assert_eq!((m["minutes"].clone(), m["name"].clone()), (json!(10), json!("green tea")));
    assert!(!named.matches("set a 10 minute timer", &types).unwrap().contains_key("name"));

    let hms = Sentence::parse("set a timer for {hours} hour[s] [and] {minutes} minute[s] [and] {seconds} second[s]").unwrap();
    let m = hms.matches("set a timer for 1 hour 30 minutes and 15 seconds", &types).unwrap();
    assert_eq!((m["hours"].clone(), m["minutes"].clone(), m["seconds"].clone()), (json!(1), json!(30), json!(15)));
}

#[test]
fn ha_intents_alternatives_and_list_slots() {
    let types = slots(&[("minutes", SlotType::Integer)]);
    let s = Sentence::parse("(set|start) [a|an|the|my] timer for {timer_minutes:minutes} minute[s]").unwrap();
    assert_eq!(s.slots(), vec!["minutes".to_string()]);
    assert!(s.matches("start my timer for 3 minutes", &types).is_some());
    assert!(s.matches("set timer for 3 minutes", &types).is_some());
    assert!(s.matches("stop my timer for 3 minutes", &types).is_none());
}

#[test]
fn sentence_syntax_errors() {
    for bad in ["timer [for", "timer for {minutes", "a | b", "timer )", "{bad slot}"] {
        assert!(Sentence::parse(bad).is_err(), "`{bad}` parsed");
    }
}
