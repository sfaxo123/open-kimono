/// The fake monotonic clock (design §5.3/R7): the harness advances time explicitly, because a
/// grant's lifetime has to be testable without waiting for it.
///
/// `advance` models "the phone slept for a while": monotonic time keeps running while the SoC is
/// powered down, which is exactly why a grant's deadline is an *absolute monotonic* deadline. If
/// the model used "time since last check" instead, a 10-minute sleep would freeze a timer that is
/// supposed to expire after 24 seconds, and the block under the grant would wake still powered.

/// Nanoseconds. Negative is legal (a deadline computed relative to a negative offset stays a
/// valid comparison against `Monotonic::now`).
pub type Nanos = i64;

/// What the model can measure: `sleep` (no CPU anywhere, so no grant check can happen) versus
/// `spin` (a core burns power, so a grant that is only a timer had better have paid for it).
pub trait Clock {
    fn now(&self) -> Nanos;
    /// Advance the clock by `d`, with the SoC off. The clock is a fake, so this returns
    /// immediately — the *point* is that grant expiries and wake budgets must account for it.
    fn sleep(&mut self, d: Nanos);
    /// Advance the clock by `d` while a core stays awake, burning `wake` budget as it goes.
    fn spin(&mut self, d: Nanos);
}

/// Host-side fake clock used by every test in this crate.
#[derive(Debug, Clone, Copy, Default)]
pub struct Monotonic {
    now: Nanos,
    /// CPU awake time since the last `sleep`, purely for reporting: if it grows while every
    /// grant is dormant, something is polling for no reason (design R12).
    awake: Nanos,
}

impl Monotonic {
    pub fn at(now: Nanos) -> Self {
        Self { now, awake: 0 }
    }
    pub fn awake_time(&self) -> Nanos {
        self.awake
    }
    pub fn reset_awake(&mut self) {
        self.awake = 0;
    }
}

impl Clock for Monotonic {
    fn now(&self) -> Nanos {
        self.now
    }
    fn sleep(&mut self, d: Nanos) {
        self.now += d;
        self.awake = 0;
    }
    fn spin(&mut self, d: Nanos) {
        self.now += d;
        self.awake += d;
    }
}
