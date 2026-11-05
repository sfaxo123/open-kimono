//! Edge-triggered thresholds with hysteresis (SERVICE-IR.md §8.3): a value
//! hovering around the threshold fires once; the trigger re-arms only after
//! the metric crosses `hysteresis_reset` in the opposite direction.

use crate::ir::{CmpOp, SensorThreshold};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SensorTrack {
    pub armed: bool,
    /// A crossing waiting out `debounce_ms` (fires only if it still holds).
    pub pending: bool,
}

pub fn satisfied(op: CmpOp, v: f64, threshold: f64) -> bool {
    match op {
        CmpOp::Lt => v < threshold,
        CmpOp::Lte => v <= threshold,
        CmpOp::Gt => v > threshold,
        CmpOp::Gte => v >= threshold,
        CmpOp::Eq => v == threshold,
    }
}

/// Has the metric crossed back far enough to re-arm?
pub fn rearmed(st: &SensorThreshold, v: f64) -> bool {
    match st.operator {
        CmpOp::Lt | CmpOp::Lte => v >= st.hysteresis_reset,
        CmpOp::Gt | CmpOp::Gte => v <= st.hysteresis_reset,
        CmpOp::Eq => v != st.threshold,
    }
}

impl SensorTrack {
    /// Initial arming from the value present at install (armed unless the
    /// condition already holds — installing below 20% does not alert).
    pub fn new(st: &SensorThreshold, initial: Option<f64>) -> Self {
        Self { armed: initial.is_none_or(|v| !satisfied(st.operator, v, st.threshold)), pending: false }
    }

    /// Feed one sample. Returns `true` when this sample is a firing candidate.
    pub fn sample(&mut self, st: &SensorThreshold, v: f64) -> bool {
        let sat = satisfied(st.operator, v, st.threshold);
        if self.pending && !sat {
            // The crossing did not hold for the debounce window: never fired.
            self.pending = false;
            self.armed = true;
            return false;
        }
        if !st.edge_only {
            return sat;
        }
        if self.armed && sat {
            self.armed = false;
            return true;
        }
        if !self.armed && !self.pending && rearmed(st, v) {
            self.armed = true;
        }
        false
    }
}
