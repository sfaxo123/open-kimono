//! Anticipation engine (A2.2): context triggers produce ask-first prompts.
//!
//! A trigger never acts. Dismissal lowers confidence; repeated dismissal
//! silences that trigger until the pattern is re-learned.

#[derive(Clone, Debug, PartialEq)]
pub enum Anticipation {
    Ask(String),
    Silent,
}

pub struct Engine {
    threshold: f64,
    confidence: std::collections::HashMap<String, f64>,
}

impl Engine {
    pub fn new() -> Self {
        Self { threshold: 0.6, confidence: std::collections::HashMap::new() }
    }

    pub fn observe(&mut self, trigger: &str, score: f64) {
        self.confidence.insert(trigger.to_string(), score.clamp(0.0, 1.0));
    }

    /// Car-Bluetooth style trigger: above threshold -> ask, never act.
    pub fn on_trigger(&self, trigger: &str, prompt: &str) -> Anticipation {
        match self.confidence.get(trigger) {
            Some(&s) if s >= self.threshold => Anticipation::Ask(prompt.to_string()),
            _ => Anticipation::Silent,
        }
    }

    /// User said no: re-fit downward. Three dismissals silence the trigger.
    pub fn dismiss(&mut self, trigger: &str) {
        if let Some(s) = self.confidence.get_mut(trigger) {
            *s = (*s - 0.25).max(0.0);
        }
    }

    pub fn confirm(&mut self, trigger: &str) {
        if let Some(s) = self.confidence.get_mut(trigger) {
            *s = (*s + 0.1).min(1.0);
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_asks_never_acts() {
        let mut e = Engine::new();
        e.observe("car_bt", 0.9);
        assert_eq!(e.on_trigger("car_bt", "Going to Tabby's?"), Anticipation::Ask("Going to Tabby's?".into()));
    }

    #[test]
    fn weak_pattern_stays_silent() {
        let mut e = Engine::new();
        e.observe("car_bt", 0.3);
        assert_eq!(e.on_trigger("car_bt", "Going to Tabby's?"), Anticipation::Silent);
    }

    #[test]
    fn dismissals_silence_trigger() {
        let mut e = Engine::new();
        e.observe("car_bt", 0.9);
        e.dismiss("car_bt");
        e.dismiss("car_bt");
        assert_eq!(e.on_trigger("car_bt", "Going to Tabby's?"), Anticipation::Silent);
    }
}
