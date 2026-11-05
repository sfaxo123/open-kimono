//! Drain analytics (A4.1): estimates from measured history only.
//!
//! A recommendation with no measured baseline is refused, never invented.
//! Estimates cite the sample window they came from.

#[derive(Clone, Debug)]
pub struct Sample {
    pub component: String,
    pub pct_per_hour: f64,
    /// When the sample was taken, on the caller's clock (ms). Zero for
    /// samples recorded without a clock; the window is then unknown.
    pub at_ms: u64,
}

pub struct Drain {
    samples: Vec<Sample>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    pub component: String,
    pub pct_per_hour: f64,
    pub samples: usize,
    /// Span from the first to the last sample (ms); 0 when unknown.
    pub window_ms: u64,
}

/// A measured span in words: `2.0h`, `25m`, `40s`.
pub fn format_window(window_ms: u64) -> String {
    if window_ms >= 3_600_000 {
        format!("{:.1}h", window_ms as f64 / 3_600_000.0)
    } else if window_ms >= 60_000 {
        format!("{}m", window_ms / 60_000)
    } else {
        format!("{}s", window_ms / 1000)
    }
}

impl Drain {
    pub fn new() -> Self {
        Self { samples: Vec::new() }
    }

    pub fn record(&mut self, component: &str, pct_per_hour: f64) {
        self.record_at(component, pct_per_hour, 0);
    }

    /// Record one measured rate, stamped on the caller's clock so the
    /// estimate can cite the window it came from.
    pub fn record_at(&mut self, component: &str, pct_per_hour: f64, at_ms: u64) {
        self.samples.push(Sample { component: component.into(), pct_per_hour, at_ms });
    }

    /// Measure-and-report: None until at least one real sample exists.
    pub fn estimate(&self, component: &str) -> Option<Estimate> {
        let xs: Vec<&Sample> = self.samples.iter().filter(|s| s.component == component).collect();
        if xs.is_empty() {
            return None;
        }
        let n = xs.len();
        let first = xs.iter().map(|s| s.at_ms).min().unwrap_or(0);
        let last = xs.iter().map(|s| s.at_ms).max().unwrap_or(0);
        Some(Estimate {
            component: component.into(),
            pct_per_hour: xs.iter().map(|s| s.pct_per_hour).sum::<f64>() / n as f64,
            samples: n,
            window_ms: last.saturating_sub(first),
        })
    }

    /// Recommendation is Some only with measured evidence behind it.
    pub fn recommend_off(&self, component: &str, hours_to_charge: f64) -> Option<String> {
        let e = self.estimate(component)?;
        let spanning = if e.window_ms > 0 {
            format!(" spanning {}", format_window(e.window_ms))
        } else {
            String::new()
        };
        Some(format!(
            "{} at ~{:.1}%/hr over {} sample(s){}; ~{:.1}% until charge in {:.1}h — consider turning it off",
            e.component, e.pct_per_hour, e.samples, spanning, e.pct_per_hour * hours_to_charge, hours_to_charge
        ))
    }
}

impl Default for Drain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_baseline_no_recommendation() {
        let d = Drain::new();
        assert!(d.estimate("mic").is_none());
        assert!(d.recommend_off("mic", 8.0).is_none());
    }

    #[test]
    fn estimate_cites_sample_count() {
        let mut d = Drain::new();
        d.record("mic", 3.0);
        d.record("mic", 5.0);
        let e = d.estimate("mic").unwrap();
        assert_eq!(e.samples, 2);
        assert!((e.pct_per_hour - 4.0).abs() < 1e-9);
        assert!(d.recommend_off("mic", 8.0).unwrap().contains("2 sample"));
    }
}
