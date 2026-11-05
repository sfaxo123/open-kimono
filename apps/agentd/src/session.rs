//! Conversation sessions (CV1): the state of one spoken conversation —
//! its turns, the entities its tools established, and the slots its last
//! turn left pending — held by agentd while the conversation lives.
//!
//! A session is born with the first turn of a wake-word conversation and
//! is named by the caller (the wakeword listener mints the id and passes
//! it on every turn). While the session lives, each new request reaches
//! the brain WITH the earlier turns as conversation history, so a
//! follow-up ("how long does it take to get there?") resolves against
//! what the conversation already established. When the session expires
//! it is gone whole: a later request carrying the same id starts from
//! nothing, and a different id never sees this session's state at all.
//!
//! Two disciplines are load-bearing:
//!
//! - Context is DATA, never instruction. History reaches the brain as
//!   ordinary user/assistant messages and the entity summary as a
//!   user-role background note; the system prompt — BRAINFIX v4's
//!   confirmation discipline included — is built exactly as it is for a
//!   sessionless request and is never touched by session state.
//! - There is no speaker identity in a session. Any voice the listener
//!   accepts inside a follow-up window is treated as the session's
//!   speaker (KEY TEST 2 code C6 — a documented limit, not a solved
//!   problem). BRAINFIX's owner-confirmation rule is the mitigation
//!   that exists; this module does not weaken it.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a session survives without a turn. The follow-up window is
/// ~10 s and a spoken turn takes ~20-30 s, so the gap between recorded
/// turns stays well under this; 90 s is the window plus a short tail,
/// not a memory.
pub const SESSION_TTL: Duration = Duration::from_secs(90);

/// A session remembers at most this many turns; older ones fall off the
/// front. A spoken conversation that needs more than twenty turns of
/// verbatim history has bigger problems than this cap.
pub const MAX_TURNS: usize = 20;

/// One exchange: what the user said, and the answer they got (`None`
/// when the run failed — the turn happened, but there is no assistant
/// reply to replay as history).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub user: String,
    pub answer: Option<String>,
}

/// The snapshot a new turn starts from: everything the session already
/// established, taken before this turn is recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionContext {
    /// Earlier turns, oldest first.
    pub turns: Vec<Turn>,
    /// Entities established by tool calls (e.g. `destination`).
    pub entities: BTreeMap<String, String>,
    /// What the previous turn left open (a blocked commit's label, an
    /// unanswered question in its answer).
    pub pending: Vec<String>,
    /// A CV2 repair awaiting its answer (the turn that asked the
    /// targeted question and the span it asked about), or `None`
    /// when no repair is open. Only a turn that consumed the repair
    /// clears it; any other turn leaves it untouched.
    pub repair: Option<crate::repair::PendingRepair>,
    /// True when this snapshot is the start of a brand-new session —
    /// either the id was never seen, or the old session had expired
    /// and was wiped. A fresh context is always empty.
    pub fresh: bool,
}

impl SessionContext {
    fn empty(fresh: bool) -> SessionContext {
        SessionContext { turns: Vec::new(), entities: BTreeMap::new(), pending: Vec::new(), repair: None, fresh }
    }
}

/// A snapshot of one live session, for the session module card:
/// everything the card displays, taken from the store's real state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub id: String,
    pub turn_count: usize,
    pub entities: BTreeMap<String, String>,
    pub pending: Vec<String>,
    pub follow_up_open: bool,
}

#[derive(Clone, Debug)]
struct Session {
    turns: Vec<Turn>,
    entities: BTreeMap<String, String>,
    pending: Vec<String>,
    repair: Option<crate::repair::PendingRepair>,
    follow_up_open: bool,
    last_activity: Instant,
}

/// The session table, shared by the agent's request threads. Cheap to
/// construct; the agent owns exactly one.
pub struct SessionStore {
    sessions: Mutex<std::collections::HashMap<String, Session>>,
    ttl: Duration,
}

impl Default for SessionStore {
    fn default() -> SessionStore {
        SessionStore::new()
    }
}

impl SessionStore {
    pub fn new() -> SessionStore {
        SessionStore::with_ttl(SESSION_TTL)
    }

    pub fn with_ttl(ttl: Duration) -> SessionStore {
        SessionStore { sessions: Mutex::new(std::collections::HashMap::new()), ttl }
    }

    fn expired(&self, session: &Session, now: Instant) -> bool {
        now.duration_since(session.last_activity) > self.ttl
    }

    /// Start (or continue) the turn for `id`: returns the context the
    /// turn must see. An expired session is wiped first and reported
    /// fresh — expiry is total, never partial. Beginning a turn counts
    /// as activity: the session's clock restarts here and again when
    /// the turn is recorded.
    pub fn begin(&self, id: &str, now: Instant) -> SessionContext {
        let mut map = self.sessions.lock().expect("session store lock poisoned");
        // Opportunistic sweep: dead sessions do not get to accumulate.
        let ttl = self.ttl;
        map.retain(|_, s| now.duration_since(s.last_activity) <= ttl);
        match map.get_mut(id) {
            Some(session) => {
                session.last_activity = now;
                SessionContext {
                    turns: session.turns.clone(),
                    entities: session.entities.clone(),
                    pending: session.pending.clone(),
                    repair: session.repair.clone(),
                    fresh: false,
                }
            }
            None => {
                map.insert(
                    id.to_string(),
                    Session { turns: Vec::new(), entities: BTreeMap::new(), pending: Vec::new(), repair: None, follow_up_open: false, last_activity: now },
                );
                SessionContext::empty(true)
            }
        }
    }

    /// Record a finished turn. `pending` REPLACES the stored list: it
    /// names what THIS turn left open, and whatever the previous turn
    /// left open was consumed by this one. Entities merge — a later
    /// turn's value for the same key wins.
    pub fn record(
        &self,
        id: &str,
        user: &str,
        answer: Option<String>,
        entities: Vec<(String, String)>,
        pending: Vec<String>,
        now: Instant,
    ) {
        let mut map = self.sessions.lock().expect("session store lock poisoned");
        let stale = map.get(id).map(|s| self.expired(s, now)).unwrap_or(false);
        if stale {
            map.remove(id);
        }
        let session = map.entry(id.to_string()).or_insert_with(|| Session {
            turns: Vec::new(),
            entities: BTreeMap::new(),
            pending: Vec::new(),
            repair: None,
            follow_up_open: false,
            last_activity: now,
        });
        session.turns.push(Turn { user: user.to_string(), answer });
        if session.turns.len() > MAX_TURNS {
            let drop = session.turns.len() - MAX_TURNS;
            session.turns.drain(..drop);
        }
        for (k, v) in entities {
            session.entities.insert(k, v);
        }
        session.pending = pending;
        session.last_activity = now;
    }

    /// Store (or clear) the session's open repair. A turn that asked
    /// the targeted question stores it; the turn that consumed the
    /// answer clears it by passing `None`. Called after `record`, so
    /// the turn's text and the repair move together in order.
    pub fn set_repair(&self, id: &str, repair: Option<crate::repair::PendingRepair>, now: Instant) {
        let mut map = self.sessions.lock().expect("session store lock poisoned");
        if let Some(session) = map.get_mut(id) {
            session.repair = repair;
            session.last_activity = now;
        }
    }

    /// Record the listener-reported follow-up window for `id`. The
    /// window belongs to the listener; this only mirrors its report
    /// into the session so the session card shows the same state the
    /// `follow_up_window` journal record carries. Creates the session
    /// when the window is reported before the first turn lands.
    pub fn set_follow_up(&self, id: &str, open: bool, now: Instant) {
        let mut map = self.sessions.lock().expect("session store lock poisoned");
        let session = map.entry(id.to_string()).or_insert_with(|| Session {
            turns: Vec::new(),
            entities: BTreeMap::new(),
            pending: Vec::new(),
            repair: None,
            follow_up_open: false,
            last_activity: now,
        });
        session.follow_up_open = open;
        session.last_activity = now;
    }

    /// The live session's card state, or `None` when there is no live
    /// session under `id` (never seen, or expired and not yet swept).
    pub fn snapshot(&self, id: &str, now: Instant) -> Option<SessionSnapshot> {
        let map = self.sessions.lock().expect("session store lock poisoned");
        let session = map.get(id)?;
        if self.expired(session, now) {
            return None;
        }
        Some(SessionSnapshot {
            id: id.to_string(),
            turn_count: session.turns.len(),
            entities: session.entities.clone(),
            pending: session.pending.clone(),
            follow_up_open: session.follow_up_open,
        })
    }

    /// Whether a live (unexpired) session exists under `id`.
    pub fn is_live(&self, id: &str, now: Instant) -> bool {
        let map = self.sessions.lock().expect("session store lock poisoned");
        map.get(id).map(|s| !self.expired(s, now)).unwrap_or(false)
    }
}
