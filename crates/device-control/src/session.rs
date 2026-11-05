//! Bounded host-side SESSION POLICY state machine for control-envelope v1.
//!
//! This module tracks ONE in-flight action's outcome discipline — it is NOT a
//! transport, pairing protocol or authentication implementation. It makes no
//! claim that any authorization exists; see [`SessionPolicy::accept_trusted_peer`]
//! for the explicit trust boundary.
//!
//! Trust boundary: connection authorization and peer identity verification are
//! an EXTERNAL trusted precondition. This module never verifies a signature,
//! token or caller bool. A result handed to [`SessionPolicy::accept_trusted_peer`]
//! is accepted on the caller's assertion that an external mechanism already
//! authorized it; this module only enforces that the asserted identity matches
//! the configured peer and rejects everything else without mutation.
//!
//! Hard limits (never loosened, no unbounded storage):
//! - at most ONE in-flight action at a time;
//! - strictly increasing nonzero u64 request ids; reuse, backwards jumps and
//!   exhaustion are rejected by comparing against the last issued id only;
//! - expected build/boot ids must be nonzero and distinct at construction;
//! - all tracking is in-process: there is no durable or global replay ledger.
//!
//! Deliberately absent — do not infer it from this module: network, runtime
//! dispatch, auth implementation, persistence, device actions, or any proof a
//! human actually performed an action.

use crate::Op;
use core::fmt;

/// Maximum held-input release plan size: key bitset for codes 0..=767 plus
/// ten pointer slots. Bounded by construction; never heap-allocated here.
pub const MAX_KEY_WORDS: usize = 12;
pub const POINTER_SLOTS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Disconnected,
    Ready,
    Pending,
    Acknowledged,
    ResultKnown,
    UnknownOutcome,
}

/// A peer identity. Construction requires a nonzero id; zero is never valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerId(u64);

impl PeerId {
    pub fn new(id: u64) -> Result<PeerId, SessionError> {
        if id == 0 {
            return Err(SessionError::ZeroIdentity);
        }
        Ok(PeerId(id))
    }
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A frame reference: boot/build it was captured under, plus a frame counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRef {
    pub boot: [u8; 16],
    pub build: [u8; 16],
    pub frame: u64,
}

/// A bounded plan describing held inputs that SHOULD be released. Returning a
/// plan is a request, not proof that any release actually happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReleasePlan {
    pub keys: [u64; MAX_KEY_WORDS],
    pub pointers: [bool; POINTER_SLOTS],
}

impl ReleasePlan {
    pub fn is_empty(&self) -> bool {
        self.keys.iter().all(|&w| w == 0) && self.pointers.iter().all(|&p| !p)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    ZeroIdentity,
    ZeroBuildId,
    BuildBootCollision,
    NotConnected,
    AlreadyConnected,
    WrongPeer,
    NoPendingAction,
    ActionInProgress,
    UnknownRequest,
    NonMonotonicRequest,
    RequestExhausted,
    StaleObservation,
    NothingToResolve,
    IdentityMismatch,
    ReferenceFrameLocked,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = match self {
            SessionError::ZeroIdentity => "peer identity must be nonzero",
            SessionError::ZeroBuildId => "expected build/boot id must be nonzero",
            SessionError::BuildBootCollision => "expected build and boot ids must differ",
            SessionError::NotConnected => "no trusted connection accepted yet",
            SessionError::AlreadyConnected => "a trusted connection is already active",
            SessionError::WrongPeer => "asserted peer does not match configured peer",
            SessionError::NoPendingAction => "no action is in flight",
            SessionError::ActionInProgress => "one action may be in flight at a time",
            SessionError::UnknownRequest => "request id does not match the in-flight action",
            SessionError::NonMonotonicRequest => "request ids must strictly increase",
            SessionError::RequestExhausted => "request id space exhausted; no id may issue",
            SessionError::StaleObservation => "observation frame is not newer than reference",
            SessionError::NothingToResolve => "no unknown outcome to resolve",
            SessionError::IdentityMismatch => {
                "observation identity does not match expected boot/build"
            }
            SessionError::ReferenceFrameLocked => "reference frame is locked for this action",
        };
        write!(f, "{m}")
    }
}

/// Snapshot for tests: every rejected transition must leave this unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub state: State,
    pub last_id: u64,
    pub inflight_op: Option<Op>,
    pub inflight_id: Option<u64>,
    pub pending_release: bool,
    pub reference_frame: Option<u64>,
    pub acked: Option<bool>,
    pub observed_newer: Option<bool>,
    /// Whether the in-flight action's request has been SENT onto the
    /// transport (DC-CLI-02D). Session-owned send evidence: it lives in
    /// the policy record, not in any executor, so dropping and
    /// reconstructing an executor cannot reset it. `None` with no
    /// in-flight action.
    pub sent: Option<bool>,
    pub connected_peer: Option<PeerId>,
    pub release_plan: Option<ReleasePlan>,
}

/// One in-flight action record. Single slot — no ledger, no HashSet.
#[derive(Clone, Copy)]
struct Inflight {
    op: Op,
    id: u64,
    reference_frame: u64,
    reference_set: bool,
    acked: bool,
    observed_newer: bool,
    /// DC-CLI-02D: true once the action's request has crossed the
    /// transport. Set only by `mark_sent`, which requires the id to be
    /// the in-flight one. A pre-send (proven-unsent) cancel is legal only
    /// while this is false — the invariant the act executor's `cancel`
    /// consults, because executor-local send state does not survive an
    /// executor reconstruction.
    sent: bool,
    held_keys: [u64; MAX_KEY_WORDS],
    held_pointers: [bool; POINTER_SLOTS],
}

/// The bounded session policy. All state is in-process only.
pub struct SessionPolicy {
    expected_build: [u8; 16],
    expected_boot: [u8; 16],
    expected_peer: PeerId,
    state: State,
    connected_peer: Option<PeerId>,
    last_id: u64,
    inflight: Option<Inflight>,
    pending_release: ReleasePlan,
}

fn is_zero(id: &[u8; 16]) -> bool {
    id.iter().all(|&b| b == 0)
}

impl SessionPolicy {
    /// Configure expected nonzero, distinct build/boot ids and the one peer.
    pub fn new(
        build: [u8; 16],
        boot: [u8; 16],
        peer: PeerId,
    ) -> Result<SessionPolicy, SessionError> {
        if is_zero(&build) || is_zero(&boot) {
            return Err(SessionError::ZeroBuildId);
        }
        if build == boot {
            return Err(SessionError::BuildBootCollision);
        }
        Ok(SessionPolicy {
            expected_build: build,
            expected_boot: boot,
            expected_peer: peer,
            state: State::Disconnected,
            connected_peer: None,
            last_id: 0,
            inflight: None,
            pending_release: ReleasePlan::default(),
        })
    }

    pub fn state(&self) -> State {
        self.state
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            state: self.state,
            last_id: self.last_id,
            inflight_op: self.inflight.map(|i| i.op),
            inflight_id: self.inflight.map(|i| i.id),
            pending_release: !self.pending_release.is_empty(),
            reference_frame: self.inflight.map(|i| i.reference_frame),
            acked: self.inflight.map(|i| i.acked),
            observed_newer: self.inflight.map(|i| i.observed_newer),
            sent: self.inflight.map(|i| i.sent),
            connected_peer: self.connected_peer,
            release_plan: self.release_plan(),
        }
    }
    pub fn release_plan(&self) -> Option<ReleasePlan> {
        if self.pending_release.is_empty() {
            None
        } else {
            Some(self.pending_release)
        }
    }

    /// TRUST BOUNDARY: accept a connection the caller asserts an EXTERNAL
    /// mechanism already authorized. This module performs no verification of
    /// that claim; it only rejects identities mismatching the configured peer.
    pub fn accept_trusted_peer(&mut self, asserted: PeerId) -> Result<(), SessionError> {
        if self.state != State::Disconnected {
            return Err(SessionError::AlreadyConnected);
        }
        if asserted != self.expected_peer {
            return Err(SessionError::WrongPeer);
        }
        self.connected_peer = Some(asserted);
        self.state = State::Ready;
        Ok(())
    }

    /// Start one action. Rejected without mutation when disconnected, another
    /// action is in flight, the id is not strictly increasing/nonzero, or the
    /// id space is exhausted. Allowed from Ready or after a prior ResultKnown.
    pub fn start_action(&mut self, op: Op, request_id: u64) -> Result<(), SessionError> {
        if self.inflight.is_some() {
            return Err(SessionError::ActionInProgress);
        }
        if self.connected_peer.is_none() {
            return Err(SessionError::NotConnected);
        }
        if self.state != State::Ready && self.state != State::ResultKnown {
            return Err(SessionError::NotConnected);
        }
        if !self.pending_release.is_empty() {
            return Err(SessionError::NothingToResolve);
        }
        if self.last_id == u64::MAX {
            return Err(SessionError::RequestExhausted);
        }
        if request_id == 0 || request_id <= self.last_id {
            return Err(SessionError::NonMonotonicRequest);
        }
        self.last_id = request_id;
        self.inflight = Some(Inflight {
            op,
            id: request_id,
            reference_frame: 0,
            reference_set: false,
            acked: false,
            observed_newer: false,
            sent: false,
            held_keys: [0; MAX_KEY_WORDS],
            held_pointers: [false; POINTER_SLOTS],
        });
        self.state = State::Pending;
        Ok(())
    }

    /// Mark the in-flight action's request as SENT (DC-CLI-02D: the
    /// session-owned send evidence). Refused without mutation unless the
    /// id names the in-flight action — a sent flag may never be stamped
    /// onto a non-in-flight or already-closed record. Once true it never
    /// goes back: an action that crossed the transport stays ambiguous
    /// for the rest of its life in this slot, regardless of which
    /// executor is driving it.
    pub fn mark_sent(&mut self, request_id: u64) -> Result<(), SessionError> {
        match self.inflight {
            Some(i) if i.id == request_id => {
                let inf = self.inflight.unwrap();
                self.inflight = Some(Inflight { sent: true, ..inf });
                Ok(())
            }
            _ => Err(SessionError::UnknownRequest),
        }
    }

    /// Record the device's reference frame for a started action. A wrong id or
    /// an out-of-state call leaves everything unchanged. Once set, the frame
    /// may only move strictly forward, and once an observation has been
    /// recorded against it, it is locked for the rest of the action — a
    /// reference frame must never be rewritten to launder a stale observation
    /// into a newer one.
    pub fn set_reference_frame(&mut self, request_id: u64, frame: u64) -> Result<(), SessionError> {
        match self.inflight {
            Some(i)
                if i.id == request_id
                    && (self.state == State::Pending || self.state == State::Acknowledged) =>
            {
                if i.observed_newer || (i.reference_set && frame <= i.reference_frame) {
                    return Err(SessionError::ReferenceFrameLocked);
                }
                let inf = self.inflight.unwrap();
                self.inflight = Some(Inflight {
                    reference_frame: frame,
                    reference_set: true,
                    ..inf
                });
                Ok(())
            }
            _ => Err(SessionError::UnknownRequest),
        }
    }

    /// Register the FULL set of inputs the in-flight action currently holds
    /// (key bitset for codes 0..=767, ten pointer slots). This replaces any
    /// previous registration for the action. The held-input release plan
    /// exposed on disconnect is exactly this set — it is never derived from
    /// the request id or any other unrelated value. A wrong id or an
    /// out-of-state call leaves everything unchanged.
    pub fn register_held_inputs(
        &mut self,
        request_id: u64,
        keys: [u64; MAX_KEY_WORDS],
        pointers: [bool; POINTER_SLOTS],
    ) -> Result<(), SessionError> {
        match self.inflight {
            Some(i)
                if i.id == request_id
                    && (self.state == State::Pending || self.state == State::Acknowledged) =>
            {
                let inf = self.inflight.unwrap();
                self.inflight = Some(Inflight {
                    held_keys: keys,
                    held_pointers: pointers,
                    ..inf
                });
                Ok(())
            }
            _ => Err(SessionError::UnknownRequest),
        }
    }

    /// Record an ACK. An ACK NEVER marks anything completed; it only moves
    /// Pending to Acknowledged for the matching in-flight id.
    pub fn record_ack(&mut self, request_id: u64) -> Result<(), SessionError> {
        match self.inflight {
            Some(i) if i.id == request_id && self.state == State::Pending => {
                let inf = self.inflight.unwrap();
                self.inflight = Some(Inflight { acked: true, ..inf });
                self.state = State::Acknowledged;
                Ok(())
            }
            Some(i) if i.id == request_id => Err(SessionError::UnknownRequest),
            _ => Err(SessionError::UnknownRequest),
        }
    }

    /// Record a result for the in-flight action. For Command/Input this is NOT
    /// proof of a UI effect; input completion additionally requires an
    /// observation with newer frame (see `record_observation`).
    pub fn record_result(&mut self, request_id: u64, ok: bool) -> Result<(), SessionError> {
        match self.inflight {
            Some(i)
                if i.id == request_id
                    && (self.state == State::Pending || self.state == State::Acknowledged) =>
            {
                let inf = self.inflight.unwrap();
                let complete = match inf.op {
                    Op::Logs | Op::Screenshot => true,
                    Op::Command => inf.acked,
                    Op::Input => inf.acked && inf.observed_newer,
                };
                if complete && ok {
                    self.inflight = None;
                    self.state = State::ResultKnown;
                }
                // A non-completing result changes nothing: a result that
                // arrives before its ACK must not fabricate the Acknowledged
                // state or strand the action — the ACK can still land and a
                // later result can still complete it.
                Ok(())
            }
            _ => Err(SessionError::UnknownRequest),
        }
    }

    /// DC-ACT-FAILURE: close the in-flight Command action on a KNOWN FAILED
    /// outcome — the device completed the command and reported failure. Per
    /// the frozen result contract, `Failure` means completed-and-reported, so
    /// the action is no longer unresolved; it is NOT a success and nothing
    /// here reports or implies one.
    ///
    /// This is a separate, narrowly typed edge, deliberately NOT a change to
    /// [`SessionPolicy::record_result`]: `record_result(request_id, false)`
    /// keeps its existing meaning for every other caller (a result arriving
    /// before its ACK must fabricate nothing and close nothing). This edge
    /// fires only when every precondition names the SAME action: the id is
    /// the in-flight record, that record's operation is [`Op::Command`], it
    /// has been ACKed, and the session is in the Acknowledged in-flight
    /// state. Anything else — a non-in-flight or mismatched id, another
    /// operation, no ACK yet, or an already-closed slot (a duplicate
    /// closure) — is rejected with `UnknownRequest` and leaves the snapshot
    /// exactly as it was.
    ///
    /// UnknownOutcome is untouched by this edge: an unknown outcome still
    /// exits only through [`SessionPolicy::resolve_unknown_outcome`], and no
    /// retry, ACK or success is fabricated here.
    pub fn record_command_failure(&mut self, request_id: u64) -> Result<(), SessionError> {
        match self.inflight {
            Some(i)
                if i.id == request_id
                    && i.op == Op::Command
                    && i.acked
                    && self.state == State::Acknowledged =>
            {
                self.inflight = None;
                self.state = State::ResultKnown;
                Ok(())
            }
            _ => Err(SessionError::UnknownRequest),
        }
    }

    /// Record a screen observation. It counts toward completion only when it
    /// matches the expected boot/build, names the in-flight id, and is newer
    /// than the action's reference frame; otherwise state is unchanged.
    pub fn record_observation(&mut self, f: FrameRef, request_id: u64) -> Result<(), SessionError> {
        match self.inflight {
            Some(i)
                if i.id == request_id
                    && (self.state == State::Pending || self.state == State::Acknowledged) => {}
            _ => return Err(SessionError::UnknownRequest),
        }
        let inf = self.inflight.unwrap();
        if f.boot != self.expected_boot || f.build != self.expected_build {
            return Err(SessionError::IdentityMismatch);
        }
        if inf.op == Op::Input && f.frame <= inf.reference_frame {
            return Err(SessionError::StaleObservation);
        }
        if inf.op == Op::Input {
            let inf = self.inflight.unwrap();
            self.inflight = Some(Inflight {
                observed_newer: true,
                ..inf
            });
        }
        Ok(())
    }

    /// Disconnect. A pending Command/Input becomes UnknownOutcome and exposes
    /// the held-input release plan registered for that action (a request, NOT
    /// an executed release; empty when nothing was registered). Safe
    /// observation actions (Logs/Screenshot) cancel instead of fabricating
    /// success, and the session stays connected and Ready. With nothing in
    /// flight the session actually disconnects. Calling this while already in
    /// UnknownOutcome changes nothing and re-exposes the same plan: an
    /// unknown outcome exits ONLY via [`SessionPolicy::resolve_unknown_outcome`].
    /// No automatic retry ever happens here.
    pub fn disconnect(&mut self) -> Option<ReleasePlan> {
        if self.state == State::UnknownOutcome {
            return Some(self.pending_release);
        }
        if self.state == State::Disconnected {
            return None;
        }
        match self.inflight {
            Some(i) if i.op == Op::Command || i.op == Op::Input => {
                let plan = ReleasePlan {
                    keys: i.held_keys,
                    pointers: i.held_pointers,
                };
                self.pending_release = plan;
                self.inflight = None;
                self.connected_peer = None;
                self.state = State::UnknownOutcome;
                Some(plan)
            }
            Some(_) => {
                self.inflight = None;
                self.state = State::Ready;
                None
            }
            None => {
                self.connected_peer = None;
                self.state = State::Disconnected;
                None
            }
        }
    }

    /// Explicit EXTERNAL trusted decision resolving an unknown outcome. This
    /// is the only way out of UnknownOutcome; nothing resolves automatically.
    pub fn resolve_unknown_outcome(&mut self) -> Result<(), SessionError> {
        if self.state != State::UnknownOutcome {
            return Err(SessionError::NothingToResolve);
        }
        self.pending_release = ReleasePlan::default();
        self.state = State::Disconnected;
        Ok(())
    }

    /// Gate check only: reconnect is permitted solely from Disconnected; an
    /// unresolved unknown outcome blocks progress until explicitly resolved.
    /// This performs no transition itself — the actual reconnection is
    /// [`SessionPolicy::accept_trusted_peer`], which re-asserts the peer
    /// identity through the external trust boundary.
    pub fn reconnect(&mut self) -> Result<(), SessionError> {
        if self.state != State::Disconnected {
            return Err(SessionError::AlreadyConnected);
        }
        Ok(())
    }
}
