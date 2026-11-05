//! The grant model (design §5.3/§5.4): what a grant *is*, and the per-call `check` that decides
//! whether a service may do a thing right now.
//!
//! Three things here are load-bearing and are the reason this crate exists:
//!
//! * **A dead grant is no authority at all.** An expired/parked/revoked grant is not a grant that
//!   "needs refreshing" — the next use needs a *new* explicit grant (R4). Keeping a dead grant
//!   matchable is how "may use the mic while navigating" quietly becomes "may use the mic".
//! * **Revocation reaches in-flight work** (`Grant::in_flight`), which is what makes "revocable"
//!   mean something more than "the next call will be refused".
//! * **Budget is debited before the action**, in the unit the budget is written in, so a caller
//!   cannot starve its own grant by sleeping inside a call and then spend the whole budget at
//!   once (R4/R12). A grant that spends power or money is never unlimited.

use crate::clock::Nanos;
use crate::scope::{Scope, Unit, Want};
use alloc::vec::Vec;

/// A grant is identified by its *number*, which is never reused: the number is the join key into
/// the journal, so a reused number would let a later reader mistake one authority for another.
pub type GrantId = u32;

/// "No grant." The zero value, present so `check` can answer without a magic number.
pub const NO_GRANT: GrantId = 0;

/// An in-flight operation number (an open stream, an unfinished transfer). Revoking a grant
/// clears these, which is the difference between revocable-as-demo and revocable-as-fact.
pub type OpId = u64;

/// How long a *thing* may be held, and how much of it. Written by the manifest author; checked
/// by the host. `Unlimited` is legal only for a grant that spends neither power nor money
/// (R4/R12): a clock read or a notification is not worth an approval tap, and never was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Budget {
    /// No ceiling. Only legal for a cap that neither spends power (a wake window, on-time, or a
    /// radio) nor can be used twice for two different purposes (R1).
    Unlimited,
    /// At most `n` actions (a count cap, e.g. `notify.post(count=20)`).
    Uses(u32),
    /// At most `n` bytes out of the device (a volume cap, e.g. `network.connect(bytes=…)`).
    Bytes(u64),
    /// On-time budget: `total` on-time per `window` (the duty-cycle form, `33%/1s`), or `max_open`
    /// as the longest *single* hold (`max=5m`). A wake lock with neither is a phone that never
    /// sleeps, which is why R4 refuses to mint it.
    OnTime {
        total: Nanos,
        window: Option<Nanos>,
        max_open: Option<Nanos>,
    },
}

impl Budget {
    /// What kind of budget this is, for reporting and for R4's "a cap that costs power must carry
    /// a budget" check.
    pub fn unit(&self) -> Unit {
        match self {
            Budget::Unlimited | Budget::Uses(_) => Unit::None,
            Budget::Bytes(_) => Unit::Bytes,
            Budget::OnTime { .. } => Unit::OnTime,
        }
    }

    /// Whether this budget alone imposes a cumulative ceiling on this capability's work.
    /// Rolling duty cycles need an absolute grant deadline unless their allowance is zero.
    pub fn bounds_power_for(&self, kind: &str) -> bool {
        match (kind, self) {
            ("network.connect" | "network.mdns", Budget::Bytes(_)) => true,
            ("model.infer" | "time.alarm", Budget::Uses(_)) => true,
            (_, Budget::OnTime { total, window, max_open }) => {
                *total >= 0
                    && max_open.map(|value| value >= 0).unwrap_or(true)
                    && window.map(|value| value > 0).unwrap_or(true)
                    && (window.is_none() || *total == 0)
            }
            _ => false,
        }
    }

    /// How much of the budget `want` would spend, *before* anything happens (R4: debit first).
    pub fn cost_of(&self, want: &Want) -> Option<Nanos> {
        match (self, want.unit(), want) {
            (Budget::OnTime { .. }, Unit::OnTime, Want::Stream { dur }) => (*dur > 0).then_some(*dur),
            // C-1 (audit 2026-10-25): a connect of *zero* bytes is still a connect — it brings the
            // modem rail up — so it must not be metered at zero and slipped through an exhausted
            // budget forever. Mirror the `dur > 0` rule above: a cost has to be positive to exist.
            (Budget::Bytes(_), Unit::Bytes, Want::Connect { bytes }) => (*bytes > 0)
                .then(|| Nanos::try_from(*bytes).ok())
                .flatten(),
            (Budget::Uses(_) | Budget::Unlimited, Unit::None, Want::Use) => Some(1),
            _ => None,
        }
    }
}

/// What the grant keeps powered while it is live (§5.6): the *power* effect is a property of the
/// grant, so that "the GPS stays powered for as long as the fence grant lives, and no longer" is a
/// rule the platform can *enforce* instead of a sentence in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerReq {
    /// Needs no powered block: costs nothing to hold.
    None,
    /// Needs the application cores awake while live (a wake lock with a deadline and a budget).
    Wake,
    /// Needs named blocks (DSP / codec / radio) powered while live.
    Rails(Vec<alloc::string::String>),
}

/// The state machine of a grant (§5.3). `Suspended` means *parked, not closed*: an idle timer
/// fired, so the grant may be resumed *once* its activity window reopens. Anything past that
/// (`Expired`, `Revoked`, `Released`) is closed for good, and the next use needs a new grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantState {
    Live,
    Suspended,
    Expired,
    Revoked,
    Released,
}

/// Whether a grant's *time* is still open at `now`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeState {
    /// Usable right now.
    Open,
    /// Exists, but its window is closed right now (idle gap, or a suspended activity window).
    Parked,
    /// Gone: deadline passed, revoked, or released. A *new* grant would be needed (R4).
    Closed,
}

/// One minted authority.
#[derive(Debug, Clone)]
pub struct Grant {
    pub id: GrantId,
    /// The *template* this grant was minted from (the manifest key), for the audit trail.
    pub template: alloc::string::String,
    /// The capability kind, e.g. `audio.capture`.
    pub kind: alloc::string::String,
    pub scope: Scope,
    /// Who holds it.
    pub service: alloc::string::String,
    /// Which generation of that service holds it (B9: a rollback revokes the old generation's
    /// grants, so `gen` has to be part of the identity for revocation to reach them).
    pub instance: u32,
    pub state: GrantState,
    /// True when only one holder may have this resource at a time (`audio.capture` on one mic,
    /// one fence's exclusive use). R12's conflict check refuses a *second* live grant on the same
    /// exclusive resource instead of letting two routines share a mic (R4's example).
    pub exclusive: bool,
    /// Absolute *monotonic* deadline. `None` only for a kind that spends neither power nor money.
    pub deadline: Option<Nanos>,
    /// Cumulative lifetime ceiling for rolling duty grants bounded by a deadline.
    pub lifetime_budget: Option<Nanos>,
    /// Lifetime on-time already debited, independent of rolling-window `spent`.
    pub lifetime_spent: Nanos,
    /// Parks (does not close) the grant after this much inactivity (R4).
    pub idle: Option<Nanos>,
    /// Last use, for the idle timer.
    pub last_use: Option<Nanos>,
    pub budget: Budget,
    /// Already-spent budget, debited *before* each action.
    pub spent: Nanos,
    /// Start of the current duty window, for `Budget::OnTime` with a `window`.
    pub window_start: Option<Nanos>,
    /// In-flight work this grant authorises. Revoking the grant must cancel these.
    pub in_flight: Vec<OpId>,
    /// What this grant keeps powered while it is live (R7).
    pub power: PowerReq,
    /// Ask the broker again before trusting this grant (R6/R11): long-lived grants go stale.
    pub needs_revalidation: bool,
    /// The journal event that minted this grant: "why did it do that?" answered by *query*.
    pub attract_id: u64,
}

impl Grant {
    /// Is the grant's *time* still open at `now`? The absolute deadline is checked first, then the
    /// idle timer (R4/R7: a grant nobody used for longer than `idle` is *parked*, not usable).
    pub fn time_state(&self, now: Nanos) -> TimeState {
        if !matches!(self.state, GrantState::Live) {
            return TimeState::Closed;
        }
        if let Some(deadline) = self.deadline {
            if now >= deadline {
                return TimeState::Closed;
            }
        }
        if let (Some(idle), Some(last)) = (self.idle, self.last_use) {
            if now.saturating_sub(last) > idle {
                return TimeState::Parked;
            }
        }
        TimeState::Open
    }

    /// Budget still available for `want` at `now`. A grant whose duty window has *rolled over*
    /// gets its on-time counter back: that is what a *per-window* budget means.
    pub fn budget_covers(&self, want: &Want, now: Nanos) -> bool {
        let spends_power = PowerReq::for_kind(&self.kind).spends_power();
        let operation_unit = match self.kind.as_str() {
            "network.connect" | "network.mdns" => Unit::Bytes,
            "audio.capture" | "audio.play" | "power.request-wake" | "sensor.gps"
            | "sensor.imu" | "display.render" | "time.wake" => Unit::OnTime,
            _ => Unit::None,
        };
        if want.unit() != operation_unit {
            return false;
        }
        // C-9 (audit 2026-10-25): a deadline bounds a *lifetime*, not a budget. `Budget::Unlimited`
        // was only refused outright when the grant had *no* deadline, so `Unlimited` plus any
        // deadline made a power-spending kind unlimited until the clock ran out — "unlimited until
        // Thursday", which is exactly what R4's "never unlimited" forbids. `mint` refuses to write
        // one; the runtime check is what has to fail closed when the mint is bypassed.
        if spends_power && matches!(self.budget, Budget::Unlimited) {
            return false;
        }
        if spends_power
            && self.deadline.is_none()
            && !self.budget.bounds_power_for(&self.kind)
        {
            return false;
        }
        let Some(cost) = self.budget.cost_of(want) else {
            return false;
        };
        if self.spent < 0 || self.lifetime_spent < 0 {
            return false;
        }
        if let (Want::Stream { dur }, Some(deadline)) = (want, self.deadline) {
            if now.checked_add(*dur).map_or(true, |end| end > deadline) {
                return false;
            }
        }
        if let Some(lifetime_budget) = self.lifetime_budget {
            if self.lifetime_spent.checked_add(cost).map_or(true, |sum| sum > lifetime_budget) {
                return false;
            }
        }
        if spends_power && self.deadline.is_some()
            && matches!(self.budget, Budget::OnTime { .. })
            && self.lifetime_budget.is_none()
        {
            return false;
        }
        match &self.budget {
            Budget::Unlimited => self.spent.checked_add(cost).is_some(),
            Budget::Uses(n) => u32::try_from(self.spent)
                .ok()
                .is_some_and(|spent| spent < *n)
                && self.spent.checked_add(cost).is_some(),
            Budget::Bytes(n) => self.spent.checked_add(cost).is_some()
                && u64::try_from(self.spent)
                    .ok()
                    .zip(u64::try_from(cost).ok())
                    .and_then(|(spent, cost)| spent.checked_add(cost))
                    .is_some_and(|total| total <= *n),
            Budget::OnTime {
                total,
                window,
                max_open,
            } => {
                if cost > *total {
                    // A request bigger than the whole budget is refused *whole*: we never
                    // silently grant "as much as is left".
                    return false;
                }
                if let Some(max_open) = max_open {
                    if cost > *max_open {
                        return false;
                    }
                }
                match window {
                    Some(window) => {
                        let spent_in_window = match self.window_start {
                            Some(start) if now >= start.saturating_add(*window) => 0,
                            _ => self.spent,
                        };
                        spent_in_window.checked_add(cost).is_some_and(|sum| sum <= *total)
                    }
                    None => self.spent.checked_add(cost).is_some_and(|sum| sum <= *total),
                }
            }
        }
    }
}

impl PowerReq {
    /// What this *kind* typically keeps powered (§5.6's derivation table). The power effect is
    /// **derived from the capability kind, never hand-written** (R11): what blew up on the first
    /// probe run was a model writing `wake = "keep", max = "5m"` for a service that wakes once a
    /// minute, because a hand-written power entry is a guess. A derived one is a *ceiling* the
    /// safety floors (R8) may throttle below, and a model that omits the power entry is refused
    /// rather than silently unpowered.
    pub fn for_kind(kind: &str) -> PowerReq {
        match kind {
            "audio.capture" | "audio.play" => PowerReq::Rails(alloc::vec![
                alloc::string::String::from("adsp"),
                alloc::string::String::from("codec"),
            ]),
            "network.connect" | "network.mdns" => PowerReq::Rails(alloc::vec![alloc::string::String::from("modem")]),
            "sensor.gps" | "sensor.imu" => PowerReq::Rails(alloc::vec![alloc::string::String::from("sensor")]),
            "display.render" => PowerReq::Rails(alloc::vec![alloc::string::String::from("display")]),
            // The three that are *only* about time: these are why `time.wake` is a budget and not
            // a permission.
            "time.wake" | "time.alarm" | "power.request-wake" | "model.infer" => PowerReq::Wake,
            _ => PowerReq::None,
        }
    }

    pub fn spends_power(&self) -> bool {
        !matches!(self, PowerReq::None)
    }
}

/// What a service asked for, in one line: `check(service, kind, scope, want, now)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request<'a> {
    pub service: &'a str,
    pub instance: u32,
    pub kind: &'a str,
    pub scope: Scope,
    pub want: Want,
    /// Monotonic time of the request (R4: grant checks are *at* monotonic intervals, and a
    /// request made during a sleep must see the time that has passed).
    pub at: Nanos,
}

impl<'a> Request<'a> {
    pub fn new(service: &'a str, kind: &'a str, scope: Scope, want: Want, at: Nanos) -> Self {
        Request {
            service,
            instance: 0,
            kind,
            scope,
            want,
            at,
        }
    }
}

/// The answer to a `check`. `Denied` and `Unavailable` are *different* answers (R5/R11): one is a
/// security decision with a reason attached, the other is "the platform could not answer, so
/// nothing happens". Both are visible; neither is permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Authorised: `grant` covers this request, budget debited, work may start.
    Allowed { grant: GrantId, op: OpId },
    /// Nothing may start: the grant's window is closed, its budget is spent, its scope does not
    /// cover the request, or there was no grant to begin with (R1: deny by default).
    Denied(Denied),
}

impl Decision {
    pub fn allowed(&self) -> bool {
        matches!(self, Decision::Allowed { .. })
    }
    pub fn denied(&self) -> bool {
        !self.allowed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    /// Nothing granted for that kind+scope at all (R1: deny by default).
    NoGrant,
    /// A grant exists but its scope does not cover this request (R3: narrowing only).
    ScopeMismatch,
    /// The grant's window is closed *right now* (its idle timer fired, or the activity stopped):
    /// the next use needs a new grant, because an activity-scoped authority is per-occurrence (R4).
    Parked,
    /// A live grant for that resource exists and is held by *someone else* (R12's conflict
    /// check): an exclusive resource does not get handed to a second holder.
    Contended,
    /// The grant's time is over (deadline or explicit revoke): the next use needs a *new* grant.
    Expired,
    /// The grant was taken back early, which also cancels in-flight work (R4).
    Revoked,
    /// The grant is live and scoped, but its budget is spent (R4: a budget is a ceiling).
    BudgetSpent,
    /// The host could not check at all (broker unreachable, stale snapshot): **fail closed** (R11).
    Unavailable,
}

/// The grant table: the grants one host process currently holds, plus the bookkeeping the design
/// says the host must be able to do *locally* (no IPC on the hot path).
#[derive(Debug, Default)]
pub struct GrantTable {
    grants: Vec<Grant>,
    next_id: GrantId,
    next_op: OpId,
    /// How many checks were refused. A number that grows while nothing is supposed to be
    /// happening means something is polling (R12), and R12's "the UI shows *what* was spent" needs
    /// this to be countable, not just logged.
    pub denied_checks: u32,
    /// Same, for checks that could not be answered at all (R11's fail-closed counter).
    pub unavailable_checks: u32,
}

impl GrantTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: GrantId) -> Option<&Grant> {
        self.grants.iter().find(|g| g.id == id)
    }

    pub fn len(&self) -> usize {
        self.grants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Grant> {
        self.grants.iter()
    }

    /// Mint a grant. Numbers are never reused, so an old journal line keeps pointing at the
    /// authority it was written about.
    pub fn issue(&mut self, mut grant: Grant) -> GrantId {
        self.next_id += 1;
        grant.id = self.next_id;
        self.grants.push(grant);
        self.next_id
    }

    /// Take a grant back early. Returns the in-flight work that has to be cancelled: a
    /// revocation that leaves an operation authorised is not a revocation (R4).
    pub fn revoke(&mut self, id: GrantId) -> Vec<OpId> {
        self.close(id, GrantState::Revoked)
    }

    /// Close deadline-expired grants and return their in-flight operations for cancellation.
    /// Hosts must call this when deadlines pass and stop each returned operation; this reference
    /// model cannot itself interrupt an external device or kernel operation.
    pub fn expire(&mut self, now: Nanos) -> Vec<OpId> {
        let mut cancel = Vec::new();
        for grant in &mut self.grants {
            if matches!(grant.state, GrantState::Live | GrantState::Suspended)
                && grant.deadline.is_some_and(|deadline| now >= deadline)
            {
                grant.state = GrantState::Expired;
                cancel.extend(core::mem::take(&mut grant.in_flight));
            }
        }
        cancel
    }

    /// Close a grant because the service stopped or the route closed: same *effect* as `revoke`,
    /// but it is *not* a revocation and must not look like one in the audit trail (R5).
    pub fn release(&mut self, id: GrantId) -> Vec<OpId> {
        self.close(id, GrantState::Released)
    }

    fn close(&mut self, id: GrantId, state: GrantState) -> Vec<OpId> {
        match self.grants.iter_mut().find(|g| g.id == id) {
            Some(grant) => {
                grant.state = state;
                core::mem::take(&mut grant.in_flight)
            }
            None => Vec::new(),
        }
    }

    /// The per-call check (§5.4), in order:
    ///
    /// 1. keep the grants whose *kind* matches and whose scope *covers* the request;
    /// 2. drop any grant whose deadline passed or whose idle timer fired — *including* one still
    ///    marked `Live`, because a dead grant is no authority (R4/R7);
    /// 3. refuse the request if its budget cannot cover the *whole* request (R4);
    /// 4. if nothing is left, say *why* (`Expired`/`Revoked`/`Parked`/`Contended`/`ScopeMismatch`
    ///    /`NoGrant`), because the audit trail needs the reason, not just the refusal (R5);
    /// 5. otherwise debit the budget *before* the action and register the in-flight work (R4).
    ///
    /// Step 2 before step 3 is why a grant that died during a sleep cannot be used by the call
    /// that wakes the phone up.
    pub fn check(&mut self, req: &Request<'_>) -> Decision {
        let now = req.at;
        let mut open: Vec<GrantId> = Vec::new();
        let mut parked: Option<GrantId> = None;
        let mut closed: Option<Denied> = None;
        let mut contended = false;
        // C-3 (audit 2026-10-25): `contended` alone only *explains* a refusal that was already
        // going to happen. Exclusivity is a one-holder rule, so the check also has to remember
        // *who* holds the exclusive resource: the earliest-minted live exclusive grant held by a
        // *different holder* (grant numbers are never reused, so the number is the arrival order).
        let mut other_holder: Option<GrantId> = None;
        let mut scope_mismatch = false;

        // 1 + 2: collect candidates, and expire on the way (a dead grant never gets revalidated).
        for grant in self.grants.iter() {
            if grant.kind != req.kind || grant.service != req.service || grant.instance != req.instance {
                // Someone *else* holding this exclusive resource is a conflict, not an absence.
                if grant.exclusive
                    && grant.kind == req.kind
                    && grant.scope.covers(&req.scope)
                    && matches!(grant.state, GrantState::Live)
                    && grant.time_state(now) == TimeState::Open
                {
                    // F-2 (Luna review 2026-10-24): the holder is `(service, instance)`, not the
                    // service name. This branch is only reached when the identity differs — the
                    // filter above already matched the kind — so every live exclusive grant of this
                    // resource found here belongs to someone else: another service, *or* another
                    // generation of the same service. Gating on the name alone let a restarted
                    // service run its own exclusive grant alongside the old generation's, putting
                    // two holders on one microphone.
                    contended = true;
                    other_holder = Some(match other_holder {
                        Some(id) if id <= grant.id => id,
                        _ => grant.id,
                    });
                }
                continue;
            }
            if !grant.scope.covers(&req.scope) {
                scope_mismatch = true;
                continue;
            }
            match grant.time_state(now) {
                TimeState::Open => open.push(grant.id),
                TimeState::Parked => {
                    if parked.is_none() {
                        parked = Some(grant.id);
                    }
                }
                // R4: a dead grant is not a grant that "needs refreshing" — it is no authority at
                // all, and the audit trail has to be able to tell *that* apart from "never asked
                // for", because a platform that cannot tell is where ambient authority creeps back.
                TimeState::Closed => {
                    let why = match grant.state {
                        GrantState::Revoked => Denied::Revoked,
                        _ => Denied::Expired,
                    };
                    if closed.is_none() {
                        closed = Some(why);
                    }
                }
            }
        }

        // 3: budget room, shortest deadline first (R12's conflict check wants the *shortest*
        // remaining authority picked first, so a wake window can be counted before it is spent).
        let mut affordable: Vec<GrantId> = Vec::new();
        for id in open.iter() {
            if let Some(grant) = self.grants.iter().find(|g| g.id == *id) {
                if grant.budget_covers(&req.want, now) {
                    affordable.push(*id);
                }
            }
        }

        let pick = affordable
            .iter()
            .copied()
            .min_by_key(|id| match self.grants.iter().find(|g| g.id == *id) {
                Some(g) => g.deadline.unwrap_or(Nanos::MAX),
                None => Nanos::MAX,
            });

        let pick = match pick {
            Some(id) => id,
            None => {
                self.denied_checks += 1;
                // Every refusal names its reason (R5), and the reasons are *different* facts: a
                // live grant with nothing left to spend is not a contended resource, and neither
                // one is "nobody ever asked for this".
                return if !open.is_empty() {
                    Decision::Denied(Denied::BudgetSpent)
                } else if contended {
                    Decision::Denied(Denied::Contended)
                } else if parked.is_some() {
                    Decision::Denied(Denied::Parked)
                } else {
                    match closed {
                        Some(why) => Decision::Denied(why),
                        None if scope_mismatch => Decision::Denied(Denied::ScopeMismatch),
                        None => Decision::Denied(Denied::NoGrant),
                    }
                };
            }
        };

        // C-3: an exclusive resource has *one* holder. If another holder's live exclusive grant
        // for this resource was minted first, that holder owns it and this request is refused —
        // even though this requester's own grant is live, scoped and affordable. (First-come, not
        // last-write: otherwise every service with a manifest line would hold the mic at once.
        // F-2: "another holder" is a different `(service, instance)`, so a new generation of the
        // same service is refused too until the old generation's grant is revoked or dies.)
        if let Some(holder) = other_holder {
            if pick > holder {
                self.denied_checks += 1;
                return Decision::Denied(Denied::Contended);
            }
        }

        // 5: debit before the action, then register the work (R4).
        let Some(grant) = self.grants.iter().find(|g| g.id == pick) else {
            self.denied_checks += 1;
            return Decision::Denied(Denied::BudgetSpent);
        };
        let Some(cost) = grant.budget.cost_of(&req.want) else {
            self.denied_checks += 1;
            return Decision::Denied(Denied::BudgetSpent);
        };
        let window_reset = match self.grants.iter().find(|g| g.id == pick) {
            Some(g) => match (&g.budget, g.window_start) {
                (Budget::OnTime {
                    window: Some(window), ..
                }, Some(start)) => now >= start.saturating_add(*window),
                _ => false,
            },
            None => false,
        };
        let spent_before = if window_reset { 0 } else { grant.spent };
        let Some(next_spent) = spent_before.checked_add(cost) else {
            self.denied_checks += 1;
            return Decision::Denied(Denied::BudgetSpent);
        };
        let next_lifetime_spent = if grant.lifetime_budget.is_some() {
            let Some(spent) = grant.lifetime_spent.checked_add(cost) else {
                self.denied_checks += 1;
                return Decision::Denied(Denied::BudgetSpent);
            };
            spent
        } else {
            grant.lifetime_spent
        };
        self.next_op += 1;
        let op = self.next_op;
        for grant in self.grants.iter_mut() {
            if grant.id == pick {
                if window_reset {
                    grant.spent = 0;
                    grant.window_start = Some(now);
                }
                grant.spent = next_spent;
                grant.lifetime_spent = next_lifetime_spent;
                grant.in_flight.push(op);
                grant.last_use = Some(now);
            }
        }
        Decision::Allowed { grant: pick, op }
    }

    /// R11's fail-closed path. The *default* is that the host answers from its cached grants, so
    /// a broker round trip on a hot path is itself a design smell — but two cases need one (a
    /// manifest that opts in with `check_needs_broker`, and a kind whose donor keeps its own
    /// table, like R6's remote provider whose authority lives on the homelab). Where a host *has*
    /// to ask, an answer it cannot get is **not** permission: `broker_reachable = false` answers
    /// `Unavailable` instead of guessing, and the two counters stay separate because a platform
    /// that cannot tell "refused" from "unreachable" cannot tell whether anything is polling (R12).
    pub fn check_requiring_broker(&mut self, req: &Request<'_>, broker_reachable: bool) -> Decision {
        if !broker_reachable {
            self.unavailable_checks += 1;
            return Decision::Denied(Denied::Unavailable);
        }
        self.check(req)
    }

    /// Finish in-flight work (a closed stream, a finished transfer). Returns whether the number
    /// was known at all: R4's in-flight cancellation has to be *observable*, or "revoked" is a
    /// comment.
    pub fn end_op(&mut self, op: OpId) -> bool {
        for grant in self.grants.iter_mut() {
            if let Some(at) = grant.in_flight.iter().position(|o| *o == op) {
                grant.in_flight.swap_remove(at);
                return true;
            }
        }
        false
    }
}
