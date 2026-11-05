//! Power policy (A4.2): aggressive, explained, pre-approved switching.
//!
//! Recommendations come from measured drain (crate::drain). Auto-switch
//! happens only for components the user pre-approved; everything else is
//! a proposal with a reason. Every decision is returned as an event line
//! for the console/journal.

use crate::drain::Drain;
use std::collections::HashSet;

pub struct PowerPolicy {
    pub drain: Drain,
    pub preapproved: HashSet<String>,
    pub events: Vec<String>,
}

impl PowerPolicy {
    pub fn new() -> Self {
        Self { drain: Drain::new(), preapproved: HashSet::new(), events: Vec::new() }
    }

    pub fn preapprove(&mut self, component: &str) {
        self.preapproved.insert(component.to_string());
    }

    /// Advice from the measured battery drain, aimed at one candidate
    /// component (or none, when nothing switchable is on and unneeded).
    /// No baseline means no recommendation: the returned line says so,
    /// and that is the whole of the advice. A pre-approved candidate is
    /// marked `OFF`, everything else `PROPOSE OFF` — the caller keeps
    /// both dry-run for anything agentd may not switch itself.
    pub fn battery_advice(&mut self, candidate: Option<&str>) -> String {
        let Some(e) = self.drain.estimate("battery") else {
            let line = "battery: no measured baseline — no recommendation".to_string();
            self.events.push(line.clone());
            return line;
        };
        let spanning = if e.window_ms > 0 {
            format!(" spanning {}", crate::drain::format_window(e.window_ms))
        } else {
            String::new()
        };
        let base = format!("battery at ~{:.1}%/hr over {} sample(s){}", e.pct_per_hour, e.samples, spanning);
        let line = match candidate {
            Some(c) if self.preapproved.contains(c) => {
                format!("OFF {c} (pre-approved, dry-run only): {base} — agentd does not touch radios, the screen or Bluetooth")
            }
            Some(c) => format!("PROPOSE OFF {c}: {base} — say yes to switch"),
            None => format!("{base}; nothing switchable is on and unneeded right now"),
        };
        self.events.push(line.clone());
        line
    }

    /// Returns the console event describing what happened and why.
    pub fn consider_off(&mut self, component: &str, hours_to_charge: f64) -> String {
        let rec = match self.drain.recommend_off(component, hours_to_charge) {
            Some(r) => r,
            None => {
                let e = format!("{component}: no measured baseline — no recommendation");
                self.events.push(e.clone());
                return e;
            }
        };
        let e = if self.preapproved.contains(component) {
            format!("OFF {component} (pre-approved): {rec}")
        } else {
            format!("PROPOSE OFF {component}: {rec} — say yes to switch")
        };
        self.events.push(e.clone());
        e
    }
}

impl Default for PowerPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unapproved_is_proposal_approved_is_switch() {
        let mut p = PowerPolicy::new();
        p.drain.record("gps", 4.0);
        assert!(p.consider_off("gps", 5.0).starts_with("PROPOSE OFF"));
        p.preapprove("gps");
        assert!(p.consider_off("gps", 5.0).starts_with("OFF gps"));
    }

    #[test]
    fn no_baseline_no_recommendation() {
        let mut p = PowerPolicy::new();
        assert!(p.consider_off("gps", 5.0).contains("no measured baseline"));
    }
}
