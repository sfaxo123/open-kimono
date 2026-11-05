//! Power by capability lifetime (design §5.6, R7): a block is powered *only* while a grant that
//! needs it is live, and the moment that grant dies the platform releases the block.
//!
//! This file exists because Redox has no power management to lean on: there is no wake lock and
//! no suspend in the kernel today (only the `time:` scheme's clocks and timeouts), so the
//! phone-OS layer has to invent the semantics rather than port them. Two rules make it testable
//! instead of aspirational:
//!
//! * **`Idle → Lp`, not `Idle → Fm`.** `Fm` (functional: clocks on, bus votes kept) is *not* a
//!   permanent state — anything that has to stay in `Fm` forever means the grant model failed to
//!   put a budget on it. `Lp` is "clocks and bus votes released, wake input still armed": the
//!   PDC/GIC wake path (design §5.6, `specs/pdc.md`) can still fire a wake event from there, so a
//!   dormant service can wake the phone *without* spinning a core.
//! * **Nothing may be held "by mistake".** `wake_source()` answers "who, if anyone, is keeping
//!   this powered?" and `None` means *wasted*: that question is answerable at all because the
//!   wake-lock table and the grant table are one table, and R12's "what is still being kept on for
//!   nothing?" has to be answerable by query, not by grep.

use crate::clock::Nanos;
use crate::grant::GrantId;
use alloc::string::String;
use alloc::vec::Vec;

/// How much power a block is allowed to burn right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    /// Clocks and bus votes released, rails down.
    Off,
    /// "Low power, maintain-full": clocks and bus votes are *released*, but the block's wake
    /// input stays armed, so an event can still wake the SoC. This is the *idle* state (R12).
    Lp,
    /// Functional: clocks on and bus votes kept. Only ever a *temporary* state (R8).
    Fm,
}

impl PowerState {
    pub fn is_on(self) -> bool {
        !matches!(self, PowerState::Off)
    }
}

/// Who is holding a domain powered, and until when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    /// The grant that paid for this power, or `None` when *nothing* justifies it — which is the
    /// wasted-power case R12 wants caught, not merely allowed.
    pub grant: Option<GrantId>,
    /// Absolute *monotonic* deadline. A wall-clock deadline would let a deep sleep, which does not
    /// advance wall time, freeze the timer and keep the block powered forever (R7).
    pub deadline: Nanos,
    /// Total on-time this hold may spend (R4: a grant that spends power always has a budget).
    pub max_on: Nanos,
    /// On-time spent so far.
    pub on_time: Nanos,
}

#[derive(Debug, Clone)]
pub struct Domain {
    pub name: String,
    pub state: PowerState,
    pub hold: Option<Hold>,
}

/// Durable per-grant on-time accounting (Luna review F-1).
///
/// The meter belongs to the *grant*, but a refusal clears the domain's `Hold` — correctly, R12
/// wants nothing kept powered with no grant behind it — and that made the meter unreachable. The
/// next call with the same grant then took the fresh-hold branch, started from zero, and got a
/// whole budget again: repeating a refused call was enough to defeat the ceiling. So the record
/// outlives the hold it closed, and `closed` marks a grant that has already been refused here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Meter {
    /// Which domain this was spent on. Domain indices are stable for the ledger's lifetime.
    domain: usize,
    grant: GrantId,
    /// On-time actually charged to this grant here. Never rewound.
    on_time: Nanos,
    /// True once this grant has been *refused on budget* here. It survives a temporary power-off
    /// ([`WakeLedger::throttle`], and a `sleep` that closes the window), and is put back only when the
    /// grant behind it is *retired* — released because it expired, was revoked, was spent, or the
    /// service stopped (R7). Throttling must not rewind it, or a thermal dip becomes a way back into
    /// a budget the grant has already spent.
    closed: bool,
}

/// The per-domain wake ledger: one table for "who is holding this up, until when, and how much
/// on-time has that cost", which is what makes the two halves of R7 (`wake = "on"` vs.
/// `wake = "on", max = "5m"`) distinguishable at all.
#[derive(Debug, Default)]
pub struct WakeLedger {
    domains: Vec<Domain>,
    /// Who is *not* asleep right now, so a spinning core can be told apart from an armed wake
    /// source. They cost completely different things: one drains a battery in an hour, the other
    /// costs roughly nothing until an interrupt lands.
    pub awake_cores: u32,
    /// C-7 (audit 2026-10-25): the ledger's own monotonic clock, advanced by `sleep`. A
    /// `Hold.deadline` is an absolute monotonic instant (R7), so comparing it needs a clock —
    /// without one the deadline was documentation, and a hold whose deadline passed during a deep
    /// sleep stayed powered forever.
    now: Nanos,
    /// F-1 (Luna review 2026-10-24): the on-time meters that outlive the holds they closed. See
    /// [`Meter`].
    meters: Vec<Meter>,
}

impl WakeLedger {
    pub fn new(domains: &[&str]) -> Self {
        WakeLedger {
            domains: domains
                .iter()
                .map(|d| Domain {
                    name: String::from(*d),
                    state: PowerState::Off,
                    hold: None,
                })
                .collect(),
            awake_cores: 0,
            now: 0,
            meters: Vec::new(),
        }
    }

    /// The ledger's monotonic instant, advanced only by `sleep`. Exposed because a deadline that
    /// cannot be read cannot be audited (R12: "who is keeping this up, and until when?").
    pub fn now(&self) -> Nanos {
        self.now
    }

    pub fn level_of(&self, domain: &str) -> PowerState {
        self.domains
            .iter()
            .find(|d| d.name == domain)
            .map(|d| d.state)
            .unwrap_or(PowerState::Off)
    }

    pub fn is_on(&self, domain: &str) -> bool {
        self.level_of(domain).is_on()
    }

    /// Who is keeping `domain` powered. `None` = nothing is, or it is held with no grant behind
    /// it: the difference between "expensive and useful" and "wasted".
    pub fn wake_source(&self, domain: &str) -> Option<GrantId> {
        self.domains
            .iter()
            .find(|d| d.name == domain)
            .and_then(|d| d.hold.as_ref())
            .and_then(|h| h.grant)
    }

    /// Take a domain up to `Fm` with a deadline and an on-time budget. Refuses a hold with no
    /// budget (R4: that is a wake window that never closes) and one with no deadline.
    ///
    /// `grant = None` is *allowed* and means something: the platform can keep a block powered
    /// because something other than a capability wants it (a boot test, a floor throttle). The
    /// point of [`WakeLedger::wasted`] is that such a hold stays *visible* — R12's "wake with no
    /// grant" is the waste to flag, and a flag needs a query to raise it.
    pub fn hold(&mut self, domain: &str, grant: Option<GrantId>, deadline: Nanos, max_on: Nanos) -> bool {
        // C-7: the deadline is absolute, so one that is already behind opens nothing. A ledger whose
        // clock has run past the deadline must refuse the hold instead of honouring a stale instant.
        if max_on <= 0 || deadline <= 0 || deadline <= self.now {
            return false;
        }
        if let Some(index) = self.domains.iter().position(|d| d.name == domain) {
            // F-1: a grant that has already been refused *on budget* here cannot open another
            // window by trying again. Retrying is not a reset; only retiring the domain is — see
            // [`WakeLedger::release`]. A throttle is not a reset either (R8).
            if grant.is_some_and(|g| self.meter(index, g).is_some_and(|m| m.closed)) {
                self.set(domain, PowerState::Lp, None);
                return false;
            }
            // C-6 (audit 2026-10-25) / F-1 (Luna review 2026-10-24): the on-time meter belongs to
            // the *grant*, not to the call. Writing `on_time: 0` unconditionally let a service
            // re-take its own hold before every wake window and spend the budget again —
            // `wake = "on", max = "5m"` turned into "on forever". C-6 carried the meter from the
            // live hold, but a refusal *clears* that hold (R12: nothing stays powered with no grant
            // behind it), so the next same-grant call took the fresh-hold branch and started from
            // zero. The carried meter is therefore the larger of the live hold and the durable
            // record, and a spent one cannot be re-entered into.
            let live = self.domains[index]
                .hold
                .as_ref()
                .filter(|h| h.grant == grant)
                .map(|h| h.on_time);
            let recorded = grant
                .and_then(|g| self.meter(index, g))
                .map(|m| m.on_time);
            let carried = match (live, recorded) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            if let Some(on_time) = carried {
                if on_time >= max_on {
                    if let Some(g) = grant {
                        self.charge(index, g, on_time, true);
                    }
                    self.set(domain, PowerState::Lp, None);
                    return false;
                }
                if let Some(g) = grant {
                    self.charge(index, g, on_time, false);
                }
                self.set(
                    domain,
                    PowerState::Fm,
                    Some(Hold {
                        grant,
                        deadline,
                        max_on,
                        on_time,
                    }),
                );
                return true;
            }
            if let Some(g) = grant {
                self.charge(index, g, 0, false);
            }
            self.set(
                domain,
                PowerState::Fm,
                Some(Hold {
                    grant,
                    deadline,
                    max_on,
                    on_time: 0,
                }),
            );
            return true;
        }
        false
    }

    fn meter(&self, domain: usize, grant: GrantId) -> Option<&Meter> {
        self.meters
            .iter()
            .find(|m| m.domain == domain && m.grant == grant)
    }

    /// Charge a grant's durable meter on this domain. The meter never rewinds, and `closed` sticks:
    /// once a grant has been refused on budget here, it stays refused until the domain is released.
    fn charge(&mut self, domain: usize, grant: GrantId, on_time: Nanos, closed: bool) {
        match self
            .meters
            .iter_mut()
            .find(|m| m.domain == domain && m.grant == grant)
        {
            Some(m) => {
                m.on_time = m.on_time.max(on_time);
                m.closed |= closed;
            }
            None => self.meters.push(Meter {
                domain,
                grant,
                on_time,
                closed,
            }),
        }
    }

    /// Spend `dur` of on-time on a domain, charging whichever grant pays for it. Returns `false`
    /// when the budget is spent, which is how a *timer* service written as
    /// `wake = "on", max = "5m"` stops being a phone that never sleeps (R4/R12): the wake window
    /// closes instead of being overspent, and the block drops to `Lp`.
    pub fn spend(&mut self, domain: &str, dur: Nanos) -> bool {
        let index = match self.domains.iter().position(|d| d.name == domain) {
            Some(i) => i,
            None => return false,
        };
        if !self.domains[index].state.is_on() {
            return false;
        }
        // Two ways to refuse: nothing is paying for this power (R12's "wake with no grant"), or the
        // budget that was paying for it has run out. Both drop the block to `Lp` instead of letting
        // a wake window run open-ended, which is the difference between a budget and a comment.
        // C-5 (audit 2026-10-25) adds a third: a spend that is not a *positive* amount of on-time.
        // A negative `dur` sailed through `on_time + dur > max_on` and subtracted from the meter,
        // so anything that can name a negative duration keeps a block powered on a spent budget —
        // the same fail-open `Budget::cost_of` refuses at the grant layer (`dur > 0`).
        let (refused, overspent) = if dur <= 0 {
            (true, false)
        } else {
            match &self.domains[index].hold {
                None => (true, false),
                Some(hold) => {
                    // C-7: past the deadline the window is closed, budget or no budget.
                    // C-8 (audit 2026-10-25): the ceiling test has to survive a cost that cannot be
                    // added at all. In `--release` `on_time + dur` wraps negative, so an unaddable
                    // overspend read as *affordable* and the budget stopped being a ceiling.
                    // Overflow is overspend.
                    let overspent = hold
                        .on_time
                        .checked_add(dur)
                        .map(|spent| spent > hold.max_on)
                        .unwrap_or(true);
                    (
                        hold.grant.is_none() || hold.deadline <= self.now || overspent,
                        overspent,
                    )
                }
            }
        };
        if refused {
            // F-1 (Luna review 2026-10-24): a refusal *on budget* is durable, not a one-call
            // setback. The hold clears either way (R12: nothing stays powered with no grant behind
            // it), but the grant's meter is marked closed, so the next hold or spend under that
            // same grant is refused too rather than starting again from zero.
            if overspent {
                let paid_by = self.domains[index].hold.as_ref().and_then(|h| h.grant);
                if let Some(g) = paid_by {
                    self.charge(index, g, 0, true);
                }
            }
            self.set(domain, PowerState::Lp, None);
            return false;
        }
        let mut charged = None;
        if let Some(d) = self.domains.iter_mut().find(|d| d.name == domain) {
            if let Some(hold) = &mut d.hold {
                hold.on_time += dur;
                charged = hold.grant.map(|g| (g, hold.on_time));
            }
            d.state = PowerState::Fm;
        } else {
            return false;
        }
        // F-1: mirror the charge into the durable meter, so the ceiling is still standing after the
        // hold that carried it is gone.
        if let Some((g, on_time)) = charged {
            self.charge(index, g, on_time, false);
        }
        true
    }

    /// Which domains are powered with **no grant** behind them: "wake with no grant", which R12
    /// calls the waste to flag rather than a feature. A host with no wake lock cannot even ask
    /// this question, so the answer had to be made *computable* somewhere.
    pub fn wasted(&self) -> Vec<&str> {
        self.domains
            .iter()
            .filter(|d| d.state == PowerState::Fm && d.hold.as_ref().map(|h| h.grant.is_none()).unwrap_or(true))
            .map(|d| d.name.as_str())
            .collect()
    }

    /// Cut a domain's power *temporarily*: drop it to `Lp` and close the wake window that is open
    /// right now, **without** retiring the grant behind it. This is the safety floors' lever (R8):
    /// `powermand` may throttle *below* a routine's budget — stop keeping a rail up once the SoC
    /// crosses a thermal ceiling — and *resume* keeping it up once the ceiling clears, while the
    /// grant that paid for the rail is still live.
    ///
    /// The distinction from [`WakeLedger::release`] is the whole point of this method, and it is a
    /// money question, not a naming one: while the rail is throttled the grant is **still live**, so
    /// its on-time meter has to survive the throttle. If cutting the power also rewound the meter,
    /// every thermal dip would hand a still-live grant a whole budget back (`max = "5m"` spent →
    /// throttle → another 5 m), and R4's budget would stop being a ceiling — the same defect F-1
    /// closed for *retried* holds, reached by a different door. `sleep` already refuses to hand a
    /// budget back on a window that closed; a throttle is a closed window too.
    pub fn throttle(&mut self, domain: &str) -> bool {
        let Some(index) = self.domains.iter().position(|d| d.name == domain) else {
            return false;
        };
        let d = &mut self.domains[index];
        let was_on = d.state.is_on();
        d.state = PowerState::Lp;
        d.hold = None;
        was_on
    }

    /// Retire a domain because the **grant behind it is gone**: it expired, was revoked, spent its
    /// budget, or the service stopped (R7). That is what puts the on-time meter back — a *new* grant
    /// on the same domain is a new authority with its own budget, which is why the ledger may clear
    /// its records here and may not during a [`WakeLedger::throttle`].
    ///
    /// A temporary power-off is *not* a retirement: call [`WakeLedger::throttle`] for that. Nothing
    /// in this crate calls this from the floor's path any more, and the two must not be conflated:
    /// a throttle that retires the grant is how a safety floor silently becomes a budget reset.
    pub fn release(&mut self, domain: &str) -> bool {
        let Some(index) = self.domains.iter().position(|d| d.name == domain) else {
            return false;
        };
        // F-1: releasing the domain is the ledger's signal that the grant behind it *died*, and
        // *that* — not another attempt, and not a throttle — is what puts the on-time meter back.
        self.meters.retain(|m| m.domain != index);
        let d = &mut self.domains[index];
        let was_on = d.state.is_on();
        d.state = PowerState::Lp;
        d.hold = None;
        was_on
    }

    /// `dt` passes with the SoC *asleep*: no core spins, so no grant is checked and no stream
    /// moves bytes — and no grant's timer freezes. Anything whose deadline fell behind during the
    /// sleep is not powered any more when the phone wakes (R7), which is the whole reason a
    /// deadline is an absolute *monotonic* deadline instead of "time since I last looked".
    pub fn sleep(&mut self, dt: Nanos) {
        // C-7 (audit 2026-10-25): a sleep has to *advance the clock the deadlines are measured
        // against* and then compare them. The promise above this function — "anything whose deadline
        // fell behind during the sleep is not powered any more when the phone wakes" — was never
        // implemented: only the on-time budget was checked, so a hold with a 10 s deadline and 60 s
        // of budget stayed `Fm` through an hour of sleep. The deadline is what makes `wake = "on"`
        // finite even when the budget is not.
        if dt > 0 {
            self.now = self.now.saturating_add(dt);
        }
        for d in self.domains.iter_mut() {
            let expired = match &d.hold {
                // C-8: same rule as `spend` — an on-time total that cannot be added has overspent.
                Some(h) => {
                    h.deadline <= self.now
                        || h.on_time
                            .checked_add(dt)
                            .map(|spent| spent > h.max_on)
                            .unwrap_or(true)
                }
                None => false,
            };
            if expired {
                d.state = PowerState::Lp;
                d.hold = None;
                // F-1: the *hold* ends here, the meter does not. A sleep closes the window; it does
                // not hand the on-time budget back.
            }
        }
    }

    /// The power bill, per grant (R12: the UI shows what was *spent*, so "who is keeping the DSP
    /// powered and why" is a lookup instead of a project-wide grep). Domains sharing one grant are
    /// merged, because a routine that needs DSP *and* codec is one wake budget, not two.
    ///
    /// It is read from the durable [`Meter`] records and **not** from the live holds, for two
    /// reasons. A bill that only lists domains powered *right now* is a power gauge, not a bill:
    /// a `throttle`, a `sleep`, and a refusal all close the hold that carried the meter, so
    /// reading holds would let the most expensive grant on the phone read as "0 s spent" the
    /// moment the floor cut its rail — while its grant stayed live and still owed the on-time.
    /// And each meter already mirrors its domain's live value (`charge` keeps them equal), so
    /// reading holds *as well* would bill the same on-time twice; one meter per `(domain, grant)`
    /// is what makes "the bill and the budget are the same number" true rather than lucky.
    pub fn spent_by_grant(&self) -> Vec<(GrantId, Nanos)> {
        let mut out: Vec<(GrantId, Nanos)> = Vec::new();
        for m in self.meters.iter() {
            match out.iter_mut().find(|(id, _)| *id == m.grant) {
                // C-8: the bill is a report, and a report that wraps negative in release says
                // "this grant cost nothing" about the most expensive grant on the phone.
                // Saturate: an overstated bill is wrong, an understated one is a lie.
                Some((_, total)) => *total = total.saturating_add(m.on_time),
                None => out.push((m.grant, m.on_time)),
            }
        }
        out
    }

    fn set(&mut self, domain: &str, state: PowerState, hold: Option<Hold>) {
        if let Some(d) = self.domains.iter_mut().find(|d| d.name == domain) {
            d.state = state;
            d.hold = hold;
        }
    }
}
