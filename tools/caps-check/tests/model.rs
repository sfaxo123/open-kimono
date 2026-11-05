//! Design-invariant tests for `docs/design/CAPS-SANDBOX.md`.
//!
//! Each test pins one design assertion (the R-numbers refer to §2.5 of that document) and asserts
//! something that could *fail* — a denial, an expiry, an exhausted budget, a refused wake window.
//! The point is not line coverage but *rule* coverage: if someone later "simplifies" the design by
//! letting a dead grant stay matchable, or by putting the broker back on a hot path, one of these
//! turns red and says which rule broke.

use caps_check::audit::{Event, EventKind, Journal};
use caps_check::clock::{Clock, Monotonic, Nanos};
use caps_check::grant::{Budget, Decision, Denied, Grant, GrantState, GrantTable, PowerReq, Request};
use caps_check::power::{PowerState, WakeLedger};
use caps_check::scope::{parse_scope, Scope, Want};

const SEC: Nanos = 1_000_000_000;

fn secs(n: Nanos) -> Nanos {
    n * SEC
}

fn millis(n: Nanos) -> Nanos {
    n * 1_000_000
}

/// A grant with the boring fields preset, so each test writes only the fields it is about.
fn grant(kind: &str, service: &str, scope: Scope, budget: Budget) -> Grant {
    Grant {
        id: 0,
        template: kind.into(),
        kind: kind.into(),
        scope,
        service: service.into(),
        instance: 0,
        state: GrantState::Live,
        exclusive: false,
        deadline: None,
        lifetime_budget: None,
        lifetime_spent: 0,
        idle: None,
        last_use: None,
        budget,
        spent: 0,
        window_start: None,
        in_flight: Vec::new(),
        power: PowerReq::None,
        needs_revalidation: false,
        attract_id: 0,
    }
}

/// A request against a *minted* grant: the grant table keys on (service, generation, kind,
/// scope), and the generation is what makes B9's "a rollback revokes the old generation's grants"
/// expressible, so a hand-built request has to say which generation is asking.
fn minted<'a>(service: &'a str, gen: u32, kind: &'a str, scope: Scope, want: Want, at: Nanos) -> Request<'a> {
    Request {
        service,
        instance: gen,
        kind,
        scope,
        want,
        at,
    }
}

/// R1 — deny by default: with no grants minted nothing is authorised, and the refusal is
/// *counted* (R12) and *recorded* (R5) rather than swallowed.
#[test]
fn a_service_with_no_grant_can_do_nothing() {
    let mut table = GrantTable::new();
    let mut clock = Monotonic::at(0);

    let decision = table.check(&Request::new(
        "nav-offline",
        "audio.capture",
        Scope::Empty,
        Want::Stream { dur: secs(1) },
        clock.now(),
    ));
    assert_eq!(
        decision,
        Decision::Denied(Denied::NoGrant),
        "R1: no ambient authority — an undeclared capability must be refused, not allowed"
    );
    assert_eq!(
        table.denied_checks, 1,
        "R12: 'is anything polling?' has to be answerable, so refusals are counted"
    );

    let mut journal = Journal::new(8);
    journal.push(Event::new(
        clock.now(),
        EventKind::GrantDenied,
        0,
        "nav-offline",
        "no-grant",
    ));
    assert_eq!(journal.len(), 1, "R5: a denial is an event, not a shrug");
    assert!(journal.get(1).is_some(), "R5: and it is retrievable by its join key");

    // The clock itself: a sleep advances monotonic time and burns no CPU (R7/R12).
    clock.sleep(secs(60));
    assert_eq!(clock.now(), secs(60), "R7: monotonic time runs through a sleep");
    assert_eq!(clock.awake_time(), 0, "R12: a sleep burns no CPU time");
}

/// R1/R3 — the *scope*, not just the capability name, decides. Two shapes that look alike in prose
/// and are nothing alike in the kernel: a sub-path inside a granted area, and a host that merely
/// *ends with* a granted host.
#[test]
fn sub_paths_are_inside_a_granted_area_but_lookalike_hosts_are_not() {
    let mut table = GrantTable::new();
    table.issue(grant(
        "state.rw",
        "nav-offline",
        Scope::Area {
            area: "nav".into(),
            read_only: true,
        },
        Budget::Unlimited,
    ));
    table.issue(grant(
        "network.connect",
        "nav-offline",
        Scope::Origin {
            host: "speech-cache.internal".into(),
            port: 443,
            scheme: "https".into(),
        },
        Budget::Bytes(4096),
    ));

    // A sub-path of a granted area is *inside* the area (R3: narrowing only).
    let inside = table.check(&Request::new(
        "nav-offline",
        "state.rw",
        Scope::Area {
            area: "nav/waypoints".into(),
            read_only: true,
        },
        Want::Use,
        0,
    ));
    assert!(inside.allowed(), "R3: a sub-path request stays inside its granted area");

    // R3's *ceiling* half: `read_only` on the grant is not a hint, it is a ceiling.
    let write = table.check(&Request::new(
        "nav-offline",
        "state.rw",
        Scope::Area {
            area: "nav/waypoints".into(),
            read_only: false,
        },
        Want::Use,
        0,
    ));
    assert!(
        matches!(write, Decision::Denied(Denied::ScopeMismatch)),
        "R3: a read-only grant never covers a write"
    );

    // R1's other half: `speech-cache.internal.evil.com` *ends with* the allowed host. Any matching
    // rule that accepts this is a hole, which is why `Origin` compares an exact triple.
    let lookalike = table.check(&Request::new(
        "nav-offline",
        "network.connect",
        Scope::Origin {
            host: "speech-cache.internal.evil.com".into(),
            port: 443,
            scheme: "https".into(),
        },
        Want::Connect { bytes: 4096 },
        0,
    ));
    assert!(
        matches!(lookalike, Decision::Denied(Denied::ScopeMismatch)),
        "R1: a host that merely *ends with* an allowed host is not allowed"
    );

    // Same host, wrong port: also a mismatch, because the grant names an exact destination.
    let wrong_port = table.check(&Request::new(
        "nav-offline",
        "network.connect",
        Scope::Origin {
            host: "speech-cache.internal".into(),
            port: 8443,
            scheme: "https".into(),
        },
        Want::Connect { bytes: 4096 },
        0,
    ));
    assert!(
        matches!(wrong_port, Decision::Denied(Denied::ScopeMismatch)),
        "R1: the grant names host *and* port, not a host prefix"
    );

    // And the exact triple *does* match, or the two refusals above would prove nothing.
    assert!(table
        .check(&Request::new(
            "nav-offline",
            "network.connect",
            Scope::Origin {
                host: "speech-cache.internal".into(),
                port: 443,
                scheme: "https".into(),
            },
            Want::Connect { bytes: 4096 },
            0,
        ))
        .allowed());
}

/// R1 — a scope that cannot be *checked* cannot be *granted*: `*`, and prose like `"cloud"`.
#[test]
fn wildcard_and_prose_scopes_grant_nothing() {
    assert_eq!(
        parse_scope("target=\"*\""),
        Scope::Empty,
        "R1: `*` is not a scope, so it authorises nothing"
    );
    assert_eq!(
        parse_scope("target=\"*.internal\""),
        Scope::Empty,
        "R1: a suffix wildcard is the same hole with a domain on it"
    );
    assert_eq!(
        parse_scope("target=\"cloud\""),
        Scope::Empty,
        "a scope that names no checkable destination grants nothing"
    );
    assert_eq!(
        parse_scope("target=\"example.com:443\", scheme=\"https\""),
        Scope::Origin {
            host: "example.com".into(),
            port: 443,
            scheme: "https".into()
        },
        "and an exact destination does"
    );
    // A plaintext destination is refused too: a permitted host over HTTP is still a leak, and a
    // grant that allows one was written by someone who did not mean to.
    assert_eq!(parse_scope("target=\"example.com:80\", scheme=\"http\""), Scope::Empty);
    // A destination is a *triple*: a bare host with no port and no scheme is not checkable, and a
    // unit-less budget is not meterable.
    assert_eq!(parse_scope("target=\"example.com\""), Scope::Empty);
    assert_eq!(caps_check::scope::parse_dur("5000000000"), None, "R4: a budget has to say its unit");
    assert_eq!(caps_check::scope::parse_dur("5m"), Some(300 * SEC));
    assert_eq!(caps_check::scope::parse_bytes("40MiB"), Some(41_943_040));
    assert_eq!(caps_check::scope::parse_dur("0s"), Some(0), "zero is a valid deny-all budget");
    assert_eq!(caps_check::scope::parse_bytes("0B"), Some(0), "zero is a valid deny-all budget");
    for invalid in ["-1s", "infs", "NaNs", "1e999s"] {
        assert_eq!(caps_check::scope::parse_dur(invalid), None, "invalid duration {invalid}");
    }
    for invalid in ["-1B", "infB", "NaNB", "1e999B"] {
        assert_eq!(caps_check::scope::parse_bytes(invalid), None, "invalid byte budget {invalid}");
    }
    assert_eq!(caps_check::scope::parse_duty("0%/1s"), Some((0, SEC)));
    assert_eq!(caps_check::scope::parse_duty("100%/1s"), Some((100, SEC)));
    assert_eq!(caps_check::scope::parse_duty("101%/1s"), None);
    assert_eq!(caps_check::scope::parse_duty("50%/0s"), None);
}

#[test]
fn power_grants_need_an_enforced_finite_bound() {
    use caps_check::manifest::{self, Manifest, UseDecl};

    for window in ["while-active", "forever"] {
        let source = format!(
            r#"{{"service":"mic","uses":[{{"cap":"audio.capture","scope":"device=\"mic0\"","window":"{window}"}}]}}"#
        );
        let parsed = manifest::manifest(&source).expect("shape parses");
        assert!(!manifest::validate(&parsed).is_empty(), "window={window} is not a finite bound");
        assert!(manifest::mint(&parsed, 0, 0).is_err(), "invalid power grant must not mint");
    }

    let mut bypass = Manifest::new("mic");
    bypass.uses.push(UseDecl {
        service: "mic".into(),
        cap: "audio.capture".into(),
        scope_text: "device=mic0".into(),
        scope: Scope::Device { device: "mic0".into() },
        window: Some("while-active".into()),
        budget: Budget::Unlimited,
    });
    assert!(manifest::mint(&bypass, 0, 0).is_err(), "mint must independently fail closed");

    let source = r#"{"service":"mic","uses":[{"cap":"audio.capture","scope":"device=\"mic0\"","window":"while-active","budget":"infs"}]}"#;
    assert!(manifest::manifest(source).is_err(), "infinite budget must not parse");

    // Zero is intentionally valid: it can be minted, but the grant cannot cover positive work.
    let zero = manifest::manifest(r#"{"service":"mic","uses":[{"cap":"audio.capture","scope":"device=\"mic0\"","budget":"0s"}]}"#).unwrap();
    let (_, grants) = manifest::mint(&zero, 0, 0).expect("zero budget is a valid deny-all grant");
    let mut table = GrantTable::new();
    for grant in grants { table.issue(grant); }
    assert!(matches!(table.check(&minted(
        "mic", 1, "audio.capture", Scope::Device { device: "mic0".into() },
        Want::Stream { dur: secs(1) }, 0,
    )), Decision::Denied(Denied::BudgetSpent)));

    let mut bypass_grant = grant(
        "audio.capture", "mic", Scope::Device { device: "mic0".into() }, Budget::Unlimited,
    );
    bypass_grant.power = PowerReq::for_kind("audio.capture");
    let mut bypass_table = GrantTable::new();
    bypass_table.issue(bypass_grant);
    assert!(matches!(bypass_table.check(&Request::new(
        "mic", "audio.capture", Scope::Device { device: "mic0".into() },
        Want::Stream { dur: secs(1) }, 0,
    )), Decision::Denied(Denied::BudgetSpent)), "runtime check must fail closed on bypassed mint");

    let rolling = manifest::manifest(r#"{"service":"mic","uses":[{"cap":"audio.capture","scope":"device=\"mic0\"","window":"while-active","budget":"33%/1s"}]}"#).unwrap();
    assert!(!manifest::validate(&rolling).is_empty(), "rolling duty needs a finite grant lifetime");
    let bounded = manifest::manifest(r#"{"service":"mic","uses":[{"cap":"audio.capture","scope":"device=\"mic0\"","window":"5m","budget":"33%/1s"}]}"#).unwrap();
    assert!(manifest::validate(&bounded).is_empty(), "finite deadline bounds a rolling duty budget");
}

#[test]
fn area_scopes_reject_path_escape_syntax_on_both_sides() {
    for invalid in ["", "/nav", "nav/", "nav//music", "nav/./music", "nav/../music"] {
        assert_eq!(parse_scope(&format!("area=\"{invalid}\"")), Scope::Empty, "invalid area {invalid:?}");
    }

    let grant_scope = Scope::Area { area: "nav".into(), read_only: true };
    for area in ["nav/../music", "/nav/music", "nav//music", "nav/./music"] {
        let request = Scope::Area { area: area.into(), read_only: true };
        assert!(!grant_scope.covers(&request), "grant must reject escaped request {area:?}");
    }
    let malformed_grant = Scope::Area { area: "nav/../music".into(), read_only: true };
    assert!(!malformed_grant.covers(&Scope::Area { area: "nav/../music".into(), read_only: true }));

    let mut table = GrantTable::new();
    table.issue(grant("state.rw", "nav", grant_scope, Budget::Unlimited));
    let escaped = table.check(&Request::new(
        "nav", "state.rw", Scope::Area { area: "nav/../music".into(), read_only: true },
        Want::Use, 0,
    ));
    assert!(matches!(escaped, Decision::Denied(Denied::ScopeMismatch)));
}

/// R4/R7 — expiry runs on **monotonic** time, and a sleep does not pause it. This is the single
/// most important test in the crate: a wall-clock timer that freezes while the SoC is asleep is how
/// "24 seconds" becomes "24 seconds of being awake".
#[test]
fn a_sleep_does_not_pause_a_grant_and_a_wall_clock_would() {
    let mut clock = Monotonic::at(0);
    let mut table = GrantTable::new();
    let mut mic = grant(
        "audio.capture",
        "mic-relay",
        Scope::Device {
            device: "wcd9340.mic0".into(),
        },
        Budget::OnTime { total: secs(24), window: None, max_open: Some(secs(24)) },
    );
    mic.deadline = Some(secs(24));
    mic.lifetime_budget = Some(secs(24));
    mic.idle = Some(secs(2));
    mic.last_use = Some(0);
    mic.exclusive = true;
    table.issue(mic);

    // 30 seconds of *sleep*: no CPU, no checks, and the deadline still passes.
    clock.sleep(secs(30));
    let after_sleep = table.check(&Request::new(
        "mic-relay",
        "audio.capture",
        Scope::Device {
            device: "wcd9340.mic0".into(),
        },
        Want::Stream { dur: secs(1) },
        clock.now(),
    ));
    assert!(
        matches!(after_sleep, Decision::Denied(Denied::Expired)),
        "R4: a grant whose time ran out during a sleep is not usable after it — and the answer \
         says *expired*, because 'ask again' would reopen a closed activity (nav-offline §7.1)"
    );
    assert_eq!(clock.awake_time(), 0, "the sleep itself cost no CPU time");
}

/// R4 — the idle timer *parks* a grant, and a parked grant is not a licence to resume later: an
/// activity-scoped authority is a per-occurrence authority.
#[test]
fn an_idle_grant_parks_and_a_parked_grant_is_not_ambient_authority() {
    let mut clock = Monotonic::at(0);
    let mut table = GrantTable::new();
    let mut mic = grant(
        "audio.capture",
        "nav-offline",
        Scope::Device {
            device: "wcd9340.mic0".into(),
        },
        Budget::OnTime { total: secs(600), window: None, max_open: Some(secs(600)) },
    );
    mic.deadline = Some(secs(600));
    mic.lifetime_budget = Some(secs(600));
    mic.idle = Some(secs(2));
    mic.last_use = Some(0);
    table.issue(mic);

    let request = |clock: &Monotonic| {
        Request::new(
            "nav-offline",
            "audio.capture",
            Scope::Device {
                device: "wcd9340.mic0".into(),
            },
            Want::Stream { dur: secs(1) },
            clock.now(),
        )
    };

    // Inside the window: allowed, and the use rewinds the idle timer.
    assert!(table.check(&request(&clock)).allowed());
    // Silence for longer than the idle timer...
    clock.sleep(secs(10));
    // ...so the *next* use is refused: the grant's window closed (R4), and it is not silently
    // reopened.
    let parked = table.check(&request(&clock));
    assert!(
        matches!(parked, Decision::Denied(Denied::Parked)),
        "R4: a grant whose activity window closed must not stay usable by reflex"
    );
    assert_eq!(table.denied_checks, 1, "R5: the refusal is countable");
}

/// R12 — the conflict check: an exclusive resource does not get handed to a second holder. This is
/// R4's example ("mic2 during navigation *and* the assistant") expressed where it is actually
/// decided, and it is the rule that stops two model-written routines sharing one microphone.
#[test]
fn two_holders_cannot_share_one_exclusive_device() {
    let mut table = GrantTable::new();
    let mut assistant_mic = grant(
        "audio.capture",
        "assistant",
        Scope::Device {
            device: "wcd9340.mic0".into(),
        },
        Budget::OnTime { total: secs(60), window: None, max_open: Some(secs(60)) },
    );
    assistant_mic.exclusive = true;
    table.issue(assistant_mic);

    // The assistant's own use is fine.
    assert!(table
        .check(&Request::new(
            "assistant",
            "audio.capture",
            Scope::Device {
                device: "wcd9340.mic0".into(),
            },
            Want::Stream { dur: secs(1) },
            0,
        ))
        .allowed());

    // A second service asking for the same exclusive device is refused, and says *why*: the answer
    // is not "no grant", which would send the reader looking for a missing manifest line.
    let second = table.check(&Request::new(
        "nav-offline",
        "audio.capture",
        Scope::Device {
            device: "wcd9340.mic0".into(),
        },
        Want::Stream { dur: secs(1) },
        0,
    ));
    assert!(
        matches!(second, Decision::Denied(Denied::Contended)),
        "R12: one microphone, one holder — a conflict is refused, not queued and not shared"
    );
}

/// R4 — revocation reaches *in-flight* work, which is the difference between "revocable" as a word
/// and as a fact (the `network_policy.rs` donor's "invalidation cannot leave an old request
/// authorized").
#[test]
fn revoking_a_grant_cancels_the_work_it_was_paying_for() {
    let mut table = GrantTable::new();
    let grant_id = table.issue(grant(
        "network.connect",
        "translator",
        Scope::Origin {
            host: "translate.example".into(),
            port: 443,
            scheme: "https".into(),
        },
        Budget::Bytes(8 * 1024 * 1024),
    ));

    let request = |at: Nanos| {
        Request::new(
            "translator",
            "network.connect",
            Scope::Origin {
                host: "translate.example".into(),
                port: 443,
                scheme: "https".into(),
            },
            Want::Connect { bytes: 64 * 1024 },
            at,
        )
    };

    let first = table.check(&request(0));
    let op = match first {
        Decision::Allowed { op, grant, .. } => {
            assert_eq!(grant, grant_id, "the answer names the grant that paid for it");
            op
        }
        other => panic!("the first transfer should be authorised, got {:?}", other),
    };

    // Now take the authority back *while the transfer is in flight*.
    let cancelled = table.revoke(grant_id);
    assert_eq!(
        cancelled.len(),
        1,
        "R4: revocation must reach the in-flight work, not just the next call"
    );
    assert!(
        !table.end_op(op),
        "R4: and the cancelled work is gone from the table, not left authorised"
    );
    let second = table.check(&request(secs(1)));
    assert!(
        matches!(second, Decision::Denied(Denied::Revoked)),
        "R4: and the *next* call must be refused too — a revocation that leaves work authorised \
         is a revocation in the manifest's prose, and prose is not a sandbox (R1)"
    );
}

/// R4 — a budget is a *ceiling*, debited before the action, and an oversize request is refused
/// whole rather than truncated to fit.
#[test]
fn budgets_are_ceilings_not_suggestions() {
    let mut table = GrantTable::new();
    let mut mic = grant(
        "audio.capture",
        "nav-offline",
        Scope::Device {
            device: "wcd9340.mic0".into(),
        },
        // 500 ms of capture per 1 s window, and no single open longer than 1.5 s.
        Budget::OnTime {
            total: millis(500),
            window: Some(SEC),
            max_open: Some(millis(1500)),
        },
    );
    mic.deadline = Some(secs(2));
    mic.lifetime_budget = Some(SEC);
    mic.window_start = Some(0);
    table.issue(mic);

    // One 2-second request does not fit a 1.5-second ceiling: refused, not truncated.
    let too_big = table.check(&Request::new(
        "nav-offline",
        "audio.capture",
        Scope::Device {
            device: "wcd9340.mic0".into(),
        },
        Want::Stream { dur: secs(2) },
        0,
    ));
    assert!(
        matches!(too_big, Decision::Denied(Denied::BudgetSpent)),
        "R4: an oversize request is refused whole, never truncated to fit"
    );

    // Opens of 400 ms every 400 ms inside a 500 ms-per-second duty cycle: the second one in any
    // window is already over the ceiling. Four would be "keeps the mic open continuously", which is
    // the thing R4 exists to refuse.
    let mut allowed = 0;
    let mut refused = 0;
    let mut at: Nanos = 0;
    while at < 2 * SEC {
        let decision = table.check(&Request::new(
            "nav-offline",
            "audio.capture",
            Scope::Device {
                device: "wcd9340.mic0".into(),
            },
            Want::Stream { dur: millis(400) },
            at,
        ));
        if decision.allowed() {
            allowed += 1;
        } else {
            assert!(
                matches!(decision, Decision::Denied(Denied::BudgetSpent)),
                "R4: past the ceiling the answer is 'budget spent', not 'as much as is left'"
            );
            refused += 1;
        }
        at += millis(400);
    }
    assert_eq!(allowed, 2, "R4: a 500 ms/s duty cycle fits two 400 ms opens per window, not five");
    assert_eq!(refused, 3, "and refuses the rest: a ceiling that allows everything is not one");
}

#[test]
fn malformed_costs_and_budget_overflow_fail_closed() {
    let scope = Scope::Device { device: "mic0".into() };
    let mut negative_stream = grant(
        "audio.capture", "mic", scope.clone(),
        Budget::OnTime { total: secs(10), window: None, max_open: None },
    );
    negative_stream.power = PowerReq::for_kind("audio.capture");
    let mut table = GrantTable::new();
    table.issue(negative_stream);
    assert_eq!(table.check(&minted(
        "mic", 0, "audio.capture", scope.clone(), Want::Stream { dur: -1 }, 0,
    )), Decision::Denied(Denied::BudgetSpent), "negative duration must never subtract from spent budget");

    let mut bytes = grant("network.connect", "net", Scope::Empty, Budget::Bytes(u64::MAX));
    assert!(!bytes.budget_covers(&Want::Connect { bytes: u64::MAX }, 0),
        "u64 byte costs that cannot fit the signed meter must be rejected");
    bytes.spent = i64::MAX;
    assert!(!bytes.budget_covers(&Want::Connect { bytes: 1 }, 0),
        "spent + cost overflow must not wrap into available byte budget");

    let mut on_time = grant(
        "audio.play", "player", Scope::Empty,
        Budget::OnTime { total: i64::MAX, window: None, max_open: None },
    );
    on_time.spent = i64::MAX;
    assert!(!on_time.budget_covers(&Want::Stream { dur: 1 }, 0),
        "on-time spent + cost overflow must fail closed");
    assert!(!on_time.budget_covers(&Want::Stream { dur: -1 }, 0),
        "negative on-time cost must fail closed");
}

#[test]
fn deadlines_and_units_bound_the_work_they_authorize() {
    use caps_check::manifest;

    let parsed = manifest::manifest(
        r#"{"service":"mic","uses":[{"cap":"audio.capture","scope":"device=\"mic0\"","window":"5m"}]}"#,
    ).unwrap();
    let (_, grants) = manifest::mint(&parsed, 0, 0).expect("finite power deadline mints bounded work");
    let mut table = GrantTable::new();
    let id = table.issue(grants.into_iter().next().unwrap());
    let long = table.check(&minted(
        "mic", 1, "audio.capture", Scope::Device { device: "mic0".into() },
        Want::Stream { dur: secs(24 * 60 * 60) }, 0,
    ));
    assert_eq!(long, Decision::Denied(Denied::BudgetSpent), "a stream may not outlive its grant deadline");
    assert_eq!(table.denied_checks, 1);

    let allowed = table.check(&minted(
        "mic", 1, "audio.capture", Scope::Device { device: "mic0".into() },
        Want::Stream { dur: secs(5 * 60) }, 0,
    ));
    let Decision::Allowed { op, .. } = allowed else { panic!("the full finite budget should fit once") };
    assert_eq!(table.check(&minted(
        "mic", 1, "audio.capture", Scope::Device { device: "mic0".into() },
        Want::Stream { dur: 1 }, 0,
    )), Decision::Denied(Denied::BudgetSpent), "concurrent/repeated work shares the cumulative ceiling");
    assert_eq!(table.denied_checks, 2, "each denied check is counted");
    assert_eq!(table.expire(secs(5 * 60)), vec![op], "deadline expiry returns in-flight work for host cancellation");
    assert_eq!(table.get(id).unwrap().state, GrantState::Expired);

    let mut suspended = grant("audio.capture", "mic", Scope::Empty, Budget::Unlimited);
    suspended.state = GrantState::Suspended;
    suspended.deadline = Some(secs(5));
    suspended.in_flight.push(42);
    let mut parked_table = GrantTable::new();
    let parked_id = parked_table.issue(suspended);
    assert_eq!(parked_table.expire(secs(5)), vec![42], "expiry must cancel parked grants too");
    assert_eq!(parked_table.get(parked_id).unwrap().state, GrantState::Expired);

    let rolling = manifest::manifest(
        r#"{"service":"mic","uses":[{"cap":"audio.capture","scope":"device=\"mic0\"","window":"5m","budget":"33%/1s"}]}"#,
    ).unwrap();
    let (_, grants) = manifest::mint(&rolling, 0, 0).unwrap();
    assert_eq!(grants[0].lifetime_budget, Some(secs(99)),
        "rolling duty has a separate lifetime ceiling derived from its finite grant lifetime");

    let on_time = grant(
        "audio.capture", "mic", Scope::Device { device: "mic0".into() },
        Budget::OnTime { total: secs(1), window: Some(secs(2)), max_open: None },
    );
    assert!(!on_time.budget_covers(&Want::Use, 0), "unit mismatch cannot be charged as one nanosecond");
    let network = grant("network.connect", "net", Scope::Empty, Budget::Bytes(16));
    assert!(!network.budget_covers(&Want::Stream { dur: secs(24 * 60 * 60) }, 0),
        "byte authority cannot authorize arbitrary-duration streams");

    let mut zero = grant(
        "audio.capture", "mic", Scope::Empty,
        Budget::OnTime { total: 0, window: None, max_open: None },
    );
    zero.power = PowerReq::None;
    assert!(!zero.budget_covers(&Want::Stream { dur: 0 }, 0), "zero budget denies even zero-duration opens");
    let mut forged = grant(
        "audio.capture", "mic", Scope::Empty,
        Budget::OnTime { total: secs(10), window: Some(SEC), max_open: None },
    );
    forged.power = PowerReq::None;
    assert!(!forged.budget_covers(&Want::Stream { dur: secs(1) }, 0),
        "runtime power policy derives from kind, not the mutable power field");
    let mut deadline_bypass = grant("audio.capture", "mic", Scope::Empty, Budget::Unlimited);
    deadline_bypass.deadline = Some(secs(5 * 60));
    deadline_bypass.power = PowerReq::for_kind("audio.capture");
    assert!(!deadline_bypass.budget_covers(&Want::Stream { dur: secs(24 * 60 * 60) }, 0),
        "a hand-built finite deadline cannot authorize work that outlives it");
}

#[test]
fn malformed_duty_budgets_are_rejected_even_with_deadlines() {
    use caps_check::manifest;

    let overflow = r#"{"service":"mic","uses":[{"cap":"audio.capture","scope":"device=\"mic0\"","budget":"50%/2000000h","window":"5m"}]}"#;
    assert!(manifest::manifest(overflow).is_err(), "duty arithmetic overflow must be a parse error");

    let mut malformed = manifest::Manifest::new("mic");
    malformed.uses.push(manifest::UseDecl {
        service: "mic".into(),
        cap: "audio.capture".into(),
        scope_text: "device=mic0".into(),
        scope: Scope::Device { device: "mic0".into() },
        window: Some("5m".into()),
        budget: Budget::OnTime { total: -1, window: Some(0), max_open: Some(-1) },
    });
    assert!(!manifest::validate(&malformed).is_empty(), "deadlines cannot make malformed budgets valid");
}

/// R7 — power by capability lifetime: the wake window exists *because* a grant is live, and dies with
/// it. A timer service whose budget is `wake = "on", max = "5m"` must *stop* being awake.
#[test]
fn a_wake_window_dies_with_the_grant_that_paid_for_it() {
    let mut ledger = WakeLedger::new(&["adsp", "codec"]);

    assert!(
        ledger.hold("adsp", Some(7), secs(600), secs(300)),
        "R7: a live grant can hold a block up"
    );
    assert_eq!(
        ledger.level_of("adsp"),
        PowerState::Fm,
        "and the block is functional while the grant lives"
    );

    // 40 wake windows of 10 s each: the 5-minute on-time budget has to run out first.
    let mut windows = 0;
    while windows < 40 && ledger.spend("adsp", secs(10)) {
        windows += 1;
    }
    assert!(
        windows < 40,
        "R4/R7: a wake budget that never runs out is a phone that never sleeps ({} windows)",
        windows
    );
    assert_eq!(
        ledger.level_of("adsp"),
        PowerState::Lp,
        "R7: once the budget is spent the block drops to idle (Lp)"
    );
    assert_eq!(
        ledger.wake_source("adsp"),
        None,
        "R12: nothing may be kept powered with no grant behind it — that *is* the bug this design \
         exists to make impossible"
    );

    // A sleep does not hold a block either: 20 minutes pass, and nothing was powered through it.
    let mut asleep = WakeLedger::new(&["apps"]);
    assert!(asleep.hold("apps", Some(3), secs(15 * 60), secs(30)));
    asleep.sleep(secs(20 * 60));
    assert_eq!(
        asleep.level_of("apps"),
        PowerState::Lp,
        "R7: 'sleeps between fires' is a testable claim, not a hope"
    );

    // R12, least power: a service that needs no powered block holds *nothing*.
    let mut idle = WakeLedger::new(&["adsp"]);
    assert!(!idle.is_on("adsp"), "R12: a grant with no power budget leaves its block off");
    assert!(
        !idle.spend("adsp", secs(1)),
        "and nothing may be spent on a domain no grant is holding"
    );
}

/// R12/R4 — the §7.2 `timer` case: a routine that *wakes* to fire is not a routine that runs forever,
/// and the model-written variant that spends 15 minutes of on-time out of a 5-minute budget must be
/// refused instead of shipped.
#[test]
fn a_timer_wakes_to_fire_instead_of_running_forever() {
    let timer = include_str!("../examples/timer.json");
    let m = caps_check::manifest::manifest(timer).expect("timer.json must parse");

    // R12: the timer wakes once a minute for two seconds, so *nothing* runs between wakes.
    assert_eq!(m.wake.as_deref(), Some("2s"), "R12: a bounded wake window, not 'keep'");
    assert_eq!(m.on_time.as_deref(), Some("5m"), "R4: and a total on-time budget");
    assert!(
        m.cores <= 4 && m.ram_kb <= 8192,
        "R12: the budget must name cores and RAM, or the supervisor cannot stop what it never \
         bounded (8 CPU cores on a 4-core phone is not budgeted)"
    );

    // The same intent written the wasteful way, as the model wrote it: a *permanent* wake window.
    let blown = include_str!("../examples/timer.blown.json");
    let blown = caps_check::manifest::manifest(blown).expect("timer.blown.json must parse");
    let errors = caps_check::manifest::validate(&blown);
    assert!(
        !errors.is_empty(),
        "R4/R12: 'wake = keep' with no on-time budget is exactly what the model must refuse"
    );
    assert!(
        errors.iter().any(|e| e.contains("wake")),
        "R5: the refusal has to name the *power* mistake, because the transparency contract is \
         about the *why* and not only the *what*"
    );

    // And the good variant's budget is a ceiling that *closes*: 450 wakes at 2 s each cannot fit
    // inside 5 minutes of on-time, so the last ~300 never happen. A clock that only ever sleeps
    // between fires is what "runs when needed" looks like from the inside.
    let mut clock = Monotonic::at(0);
    let mut ledger = WakeLedger::new(&["apps"]);
    assert!(ledger.hold("apps", Some(1), secs(15 * 60), secs(5 * 60)));
    let mut wakes = 0;
    while wakes < 450 && ledger.spend("apps", secs(2)) {
        clock.spin(secs(2));
        wakes += 1;
        clock.sleep(secs(58));
    }
    assert!(
        wakes < 450,
        "R4: a 5-minute on-time budget that survives 450 wakes is not a budget ({} fired)",
        wakes
    );
    assert_eq!(
        clock.awake_time(),
        0,
        "R12: and between wakes the SoC went back to sleep, which is the part the §7.2 probe \
         could not answer (the CPU-time counter reads 0 because nothing spun)"
    );
}

/// R11/R7/R8 — the power effect is *derived from the capability kind*, never hand-written by a model
/// (that is what blew up on the first probe), and the safety floors can still deny it.
#[test]
fn power_is_derived_from_the_kind_and_the_floor_can_deny_it() {
    // R11: "the `power` field is derived from the capability kind, not hand-written".
    assert_eq!(
        PowerReq::for_kind("audio.capture"),
        PowerReq::Rails(vec!["adsp".into(), "codec".into()]),
        "R11: capture holds DSP + codec for its lifetime, and that is what the derivation says"
    );
    assert_eq!(
        PowerReq::for_kind("state.rw"),
        PowerReq::None,
        "R12: a data area costs no power, and saying it does is how the wake table rots"
    );

    // R11's other half: an *unbounded* power request is a manifest error, so the §7.3 "always on,
    // 24/7 wake window" variant is refused before it can ship.
    let bad = include_str!("../examples/battery-watcher.bad.json");
    let bad = caps_check::manifest::manifest(bad).expect("the bad example must parse");
    let errors = caps_check::manifest::validate(&bad);
    assert!(!errors.is_empty(), "R4: the 24/7-wake + always-on-mic variant is unimplementable");

    // R8: the floor outranks the routine. A domain powered with *no grant* behind it is wasted
    // power, and the model has to be able to name it instead of merely tolerating it.
    let mut ledger = WakeLedger::new(&["adsp"]);
    assert!(
        ledger.hold("adsp", None, secs(60), secs(30)),
        "the platform *can* power a block with no grant: that is what has to be visible"
    );
    assert_eq!(
        ledger.wake_source("adsp"),
        None,
        "R12: `wake_source` answers None for it — 'wasted', not 'allowed'"
    );
    assert_eq!(ledger.wasted(), vec!["adsp"], "R12: so the §5.7 probe's second test line is a query");
    assert!(!ledger.spend("adsp", secs(1)), "R4: and nothing may be charged to a grant that does not exist");
    assert!(ledger.release("adsp"), "R7: and 'stop' means the power actually drops");
    assert!(ledger.wasted().is_empty(), "R12: with nothing behind it, nothing is held");
}

/// R5 — the audit trail is *bounded*, and overflowing it is visible instead of silent. A trail that
/// quietly overwrites would let the journal look complete while decisions had been dropped.
#[test]
fn the_journal_is_bounded_and_admits_when_it_is_full() {
    let mut journal = Journal::new(4);
    for _ in 0..40 {
        journal.push(Event::new(
            0,
            EventKind::GrantDenied,
            3,
            "nav-offline",
            "budget-exhausted",
        ));
    }
    assert_eq!(journal.len(), 4, "R5: the ring is bounded by size, so it cannot grow forever");
    assert_eq!(journal.total(), 40, "R5: and the count is not the same thing as the length");
    assert_eq!(journal.dropped(), 36);
    assert_eq!(
        journal.dropped_of(EventKind::GrantDenied),
        36,
        "R5: what got lost stays countable, so 'the trail is complete' remains a checkable claim"
    );
    assert_eq!(journal.tail(2).len(), 2, "R12: the visible feed is a fixed-size tail");
    assert!(journal.get(1).is_none() && journal.get(40).is_some(), "R5: sequence numbers are never reused");
}

/// R5 — one decision, one event, with a reason. The journal and the counter have to *agree*, or
/// "the system says what it switched and why" (VISION §4) is a slogan with a counter next to it.
#[test]
fn every_decision_is_one_event_and_the_trail_agrees_with_the_counter() {
    let mut table = GrantTable::new();
    let mut journal = Journal::new(64);
    let mut clock = Monotonic::at(0);

    let mut mic = grant(
        "audio.capture",
        "nav-offline",
        Scope::Device {
            device: "wcd9340.mic0".into(),
        },
        Budget::OnTime { total: secs(15), window: None, max_open: Some(secs(1)) },
    );
    mic.idle = Some(secs(2));
    mic.last_use = Some(0);
    table.issue(mic);

    let mut decisions = 0;
    let mut denials = 0;
    for _ in 0..40 {
        let decision = table.check(&Request::new(
            "nav-offline",
            "audio.capture",
            Scope::Device {
                device: "wcd9340.mic0".into(),
            },
            Want::Stream { dur: millis(500) },
            clock.now(),
        ));
        let kind = if decision.allowed() {
            EventKind::GrantUsed
        } else {
            denials += 1;
            EventKind::GrantDenied
        };
        journal.push(Event::new(clock.now(), kind, 1, "nav-offline", "model:check"));
        decisions += 1;
        clock.spin(millis(500));
    }

    assert_eq!(decisions, 40, "R5: every check produces exactly one event");
    assert_eq!(journal.len(), 40, "R5: and none of them were dropped on the floor");
    assert_eq!(
        journal.iter().filter(|e| e.reason.is_empty()).count(),
        0,
        "R5: an event with no reason is a log line, not an explanation"
    );
    assert!(denials > 0, "R4: the budget has to actually run out inside 40 calls");
    assert_eq!(
        denials as u32, table.denied_checks,
        "R5/R12: the trail and the refusal counter must agree, or neither is evidence"
    );
}

/// R11 — the broker must not be needed on the hot path, and where a host *does* have to ask, it
/// fails **closed**. `Unavailable` and `Denied` are different answers, and neither is permission.
#[test]
fn a_host_that_cannot_check_fails_closed() {
    let mut table = GrantTable::new();
    let mut mic = grant(
        "audio.capture",
        "nav-offline",
        Scope::Device {
            device: "wcd9340.mic0".into(),
        },
        Budget::OnTime { total: secs(10), window: None, max_open: Some(secs(10)) },
    );
    mic.needs_revalidation = true;
    table.issue(mic);

    let request = || {
        Request::new(
            "nav-offline",
            "audio.capture",
            Scope::Device {
                device: "wcd9340.mic0".into(),
            },
            Want::Stream { dur: secs(1) },
            0,
        )
    };

    // The broker answers: the grant is live, so the work may start.
    assert!(table.check_requiring_broker(&request(), true).allowed());

    // The broker does *not* answer: the same request is refused. If these two answers were the same
    // answer, "the host caches a *revoked* grant as if it were live" would be a design feature
    // (R6), and R11's "cannot check is never 'allowed'" would be prose.
    let blind = table.check_requiring_broker(&request(), false);
    assert!(
        matches!(blind, Decision::Denied(Denied::Unavailable)),
        "R11: 'could not ask' is never 'allowed'"
    );
    assert_eq!(
        table.unavailable_checks, 1,
        "R11: and the two cases are counted separately, because a platform that cannot tell \
         'refused' from 'unreachable' cannot tell whether anything is polling (R12)"
    );
}

/// R10/R9 — what a service *declares* is the ceiling on what it can *use*, both at build time and
/// at runtime. The two §7.3 example manifests are machine-checkable: the bad one mints nothing.
#[test]
fn a_manifest_that_asks_for_the_undeliverable_is_refused() {
    let good = caps_check::manifest::manifest(include_str!("../examples/battery-watcher.ok.json"))
        .expect("the good example must parse");
    let errors = caps_check::manifest::validate(&good);
    assert!(errors.is_empty(), "the good example must validate, got {:?}", errors);

    let bad = caps_check::manifest::manifest(include_str!("../examples/battery-watcher.bad.json"))
        .expect("the bad example must parse");
    let errors = caps_check::manifest::validate(&bad);
    assert!(
        errors.len() >= 3,
        "R10: the bad example breaks three separate rules (mic scope, cloud scope, wake), got {:?}",
        errors
    );
    assert!(
        errors.iter().any(|e| e.contains("scope")),
        "R1: a wildcard scope is refused by name, because `*` is the hole that AWS-S3-style \
         buckets made famous"
    );
    assert!(
        errors.iter().any(|e| e.contains("wake")),
        "R4: and so is an unbounded power request"
    );
    assert!(
        caps_check::manifest::mint(&bad, 0, 0).is_err(),
        "R4: an unvalidatable manifest mints *no* grants — R12's narrowest-first rule has to be \
         enforced where the model writes, not only in a review that nobody reads"
    );

    // R10, second clause: a *valid* manifest is still a ceiling, not a floor. The good example
    // declares one data area and one destination; anything else must be refused at runtime.
    let (last_id, grants) = caps_check::manifest::mint(&good, 0, 0).expect("a valid manifest mints grants");
    assert_eq!(last_id, 4, "one grant per declared need");
    let mut table = GrantTable::new();
    for grant in grants {
        table.issue(grant);
    }
    // A *rolled-back* generation must not keep the old generation's authority (B9).
    let rolled_back = table.check(&minted(
        "battery-watcher",
        2,
        "state.rw",
        Scope::Area {
            area: "state".into(),
            read_only: true,
        },
        Want::Use,
        0,
    ));
    assert!(
        matches!(rolled_back, Decision::Denied(Denied::NoGrant)),
        "B9: authority is per generation, so a rollback cannot leave the old one's grants live"
    );
    let other_area = table.check(&minted(
        "battery-watcher",
        1,
        "state.rw",
        Scope::Area {
            area: "music/lyrics".into(),
            read_only: true,
        },
        Want::Use,
        0,
    ));
    assert!(
        matches!(other_area, Decision::Denied(Denied::ScopeMismatch)),
        "R10: a per-service data area the service does not declare is not reachable through it"
    );
    let other_host = table.check(&minted(
        "battery-watcher",
        1,
        "network.connect",
        Scope::Origin {
            host: "home-server.example".into(),
            port: 8443,
            scheme: "https".into(),
        },
        Want::Connect { bytes: 1024 },
        0,
    ));
    assert!(
        matches!(other_host, Decision::Denied(Denied::ScopeMismatch)),
        "R1: an exact destination means *that* destination, not a category of destinations"
    );
    let mic = table.check(&minted(
        "battery-watcher",
        1,
        "audio.capture",
        Scope::Empty,
        Want::Use,
        0,
    ));
    assert!(
        matches!(mic, Decision::Denied(Denied::NoGrant)),
        "R10: `audio.capture` is not in a valid manifest, so it is not usable either"
    );
}

/// R1/R10 — the catalogue is closed, so an invented capability is a manifest error. A model that
/// writes `mic.selftest` or `shell.exec` gets a build error, not a hole with a name in it.
#[test]
fn an_invented_capability_is_a_manifest_error() {
    let invented = r#"{ "service": "x", "uses": [{ "cap": "shell.exec" }] }"#;
    let m = caps_check::manifest::manifest(invented).expect("parse");
    let errors = caps_check::manifest::validate(&m);
    assert!(
        errors.iter().any(|e| e.contains("shell.exec")),
        "R10: `shell.exec` is not in the catalogue, and the catalogue is the ceiling"
    );

    // R1: and so is a *prose* scope, which is the other half of what the model wrote in §7.3. A
    // scope that names nothing checkable would authorise every destination, which is why `kind =
    // "remote"` needs both a named relay host and a `network.connect` grant to it.
    let vague = r#"{ "service": "y", "engine": "tinywasm", "kind": "remote", "uses": [{ "cap": "network.connect", "scope": "cloud" }] }"#;
    let m = caps_check::manifest::manifest(vague).expect("parse");
    let errors = caps_check::manifest::validate(&m);
    assert!(
        errors.iter().any(|e| e.contains("cloud")),
        "R1: `target=\"cloud\"` names nothing checkable, so it must not grant everything"
    );
    assert!(
        errors.iter().any(|e| e.contains("remote")),
        "R12: a service that runs off-device has to say *which* off-device box it uses"
    );
}

// ---------------------------------------------------------------------------
// Cursor audit 2026-10-25 (`CURSOR-AUDIT-20261025.md` §2, findings C-1..C-9).
// One test per confirmed finding, in the audit's own trace, pinning the fixed
// behaviour. Each was written *before* its fix and observed red.
// ---------------------------------------------------------------------------

/// C-1 — an exhausted `Bytes` budget must not admit zero-byte connects forever.
/// The suite already refuses a zero-duration open on a spent on-time budget
/// (`deadlines_and_units_bound_the_work_they_authorize`); the byte side had no
/// equivalent, so a spent grant could still open the modem rail an unlimited
/// number of times.
#[test]
fn zero_byte_connect_cannot_ride_a_spent_budget() {
    let mut t = GrantTable::new();
    t.issue(grant("network.connect", "net", Scope::Empty, Budget::Bytes(16)));
    assert!(
        t.check(&Request::new("net", "network.connect", Scope::Empty, Want::Connect { bytes: 16 }, 0))
            .allowed(),
        "the budget covers the first, full-size connect"
    );
    assert_eq!(
        t.check(&Request::new("net", "network.connect", Scope::Empty, Want::Connect { bytes: 0 }, 1)),
        Decision::Denied(Denied::BudgetSpent),
        "C-1: a spent byte budget denies even a zero-byte connect — a connect is a *use* of the \
         radio, and R4's debit-first rule has to refuse it, not meter it at zero"
    );
}

/// C-2 — plain manifest syntax must not multiply a budget. Two `uses` entries with the same
/// `(cap, scope)` mint two grants, and `check` falls over to the second once the first is spent;
/// `Manifest::uses()` still shows only the first, so the reviewer sees half the authority.
#[test]
fn duplicate_uses_entries_are_refused_not_a_double_budget() {
    let m = caps_check::manifest::manifest(
        r#"{"service":"n","uses":[
          {"cap":"network.connect","scope":"target=\"a.example:443\", scheme=\"https\"","budget":"1KiB"},
          {"cap":"network.connect","scope":"target=\"a.example:443\", scheme=\"https\"","budget":"1KiB"}]}"#,
    )
    .expect("the manifest parses");
    let errors = caps_check::manifest::validate(&m);
    assert!(
        !errors.is_empty(),
        "C-2: duplicate (cap,scope) must be refused — one authority, one budget"
    );
    assert!(
        errors.iter().any(|e| e.contains("duplicate")),
        "R5: the refusal has to name the reason, and the reason is a duplicate declaration"
    );
    assert!(
        caps_check::manifest::mint(&m, 0, 0).is_err(),
        "C-2: a manifest with that error mints nothing, so 2 x 1KiB cannot become 2KiB of authority"
    );

    // The same capability under *different* scopes stays legal: two destinations, two grants.
    let distinct = caps_check::manifest::manifest(
        r#"{"service":"n","uses":[
          {"cap":"network.connect","scope":"target=\"a.example:443\", scheme=\"https\"","budget":"1KiB"},
          {"cap":"network.connect","scope":"target=\"b.example:443\", scheme=\"https\"","budget":"1KiB"}]}"#,
    )
    .expect("the manifest parses");
    assert!(
        caps_check::manifest::validate(&distinct).is_empty(),
        "narrowing is per-scope: the same cap for two exact destinations is not a duplicate"
    );
}

/// C-3 — exclusivity has to be enforced when *both* services hold a grant. The existing
/// `two_holders_cannot_share_one_exclusive_device` only covers a second service with no grant at
/// all; with a grant of its own, `check` marked `contended` and then picked that grant anyway,
/// so two holders ran on one microphone.
#[test]
fn two_holders_cannot_both_run_on_one_exclusive_device() {
    let mic = || Scope::Device {
        device: "wcd9340.mic0".into(),
    };
    let mut table = GrantTable::new();
    let mut assistant = grant(
        "audio.capture",
        "assistant",
        mic(),
        Budget::OnTime { total: secs(60), window: None, max_open: Some(secs(60)) },
    );
    assistant.exclusive = true;
    table.issue(assistant);

    // The second service *has* its own exclusive grant for the same device — the case the audit
    // found: `contended` was set, and then ignored because something was affordable.
    let mut nav = grant(
        "audio.capture",
        "nav-offline",
        mic(),
        Budget::OnTime { total: secs(60), window: None, max_open: Some(secs(60)) },
    );
    nav.exclusive = true;
    table.issue(nav);

    assert!(
        table
            .check(&Request::new("assistant", "audio.capture", mic(), Want::Stream { dur: secs(1) }, 0))
            .allowed(),
        "the first holder keeps the device it already has"
    );
    let second = table.check(&Request::new(
        "nav-offline",
        "audio.capture",
        mic(),
        Want::Stream { dur: secs(1) },
        0,
    ));
    assert_eq!(
        second,
        Decision::Denied(Denied::Contended),
        "C-3: one microphone, one holder — a live exclusive grant held by *someone else* refuses \
         the second holder even when its own grant is live and affordable"
    );
}

/// C-4 — an unknown `mode=` value must be an error, never read-write. R9: "an unknown shape is an
/// error, never a shrug". `read_only` was computed as `matches!(get("mode"), Some("ro"))`, so every
/// plausible typo (`read-only`, `readonly`, `RO`, `r`) silently bought *write* authority — more than
/// the author asked for — and the R3 ceiling then let that grant cover read-write requests.
#[test]
fn an_unknown_mode_is_refused_not_read_write() {
    for typo in ["read-only", "readonly", "RO", "r", "write", ""] {
        assert_eq!(
            parse_scope(&format!(r#"area="nav", mode="{typo}""#)),
            Scope::Empty,
            "C-4: `mode={typo:?}` is not a mode the host can check, so it must grant nothing"
        );
    }
    // The modes that *are* defined keep working, including the no-mode default.
    assert_eq!(
        parse_scope(r#"area="nav", mode="ro""#),
        Scope::Area { area: "nav".into(), read_only: true },
        "R3: `mode=ro` is still a read-only ceiling"
    );
    assert_eq!(
        parse_scope(r#"area="nav", mode="rw""#),
        Scope::Area { area: "nav".into(), read_only: false },
        "`mode=rw` is the explicit form of the default"
    );
    assert_eq!(
        parse_scope(r#"area="nav""#),
        Scope::Area { area: "nav".into(), read_only: false },
        "no `mode=` at all is unchanged behaviour"
    );

    // And the manifest says so out loud instead of minting a write grant from a read-only intent.
    let m = caps_check::manifest::manifest(
        r#"{"service":"n","uses":[{"cap":"state.rw","scope":"area=\"nav\", mode=\"read-only\""}]}"#,
    )
    .expect("the manifest parses");
    assert!(
        !caps_check::manifest::validate(&m).is_empty(),
        "C-4: a typo'd `mode=` has to be a manifest error, not a widening of the grant"
    );
    assert!(
        caps_check::manifest::validate(&m)
            .iter()
            .any(|e| e.contains("mode")),
        "R5: the error has to name the mistake — an unknown `mode=`, not just an empty scope"
    );
    assert!(
        caps_check::manifest::mint(&m, 0, 0).is_err(),
        "and a manifest with that error mints nothing"
    );
}

/// C-5 — `WakeLedger::spend` never checked the sign of `dur`, so a negative spend passed the
/// `on_time + dur > max_on` test and *subtracted* from the meter: a service that can name a
/// negative duration can keep a domain powered forever on a spent on-time budget.
#[test]
fn a_negative_spend_cannot_refund_a_spent_on_time_budget() {
    let mut l = WakeLedger::new(&["adsp"]);
    assert!(l.hold("adsp", Some(1), secs(60), secs(5)));
    assert!(l.spend("adsp", secs(5)), "the budget is exactly enough for one 5 s hold");
    assert!(
        !l.spend("adsp", -secs(5)),
        "C-5: a negative spend must be refused, not credited back to the meter"
    );
    assert!(
        !l.spend("adsp", 0),
        "a zero-length spend is not a use of the block either"
    );
    assert!(
        !l.spend("adsp", secs(1)),
        "and the budget stays spent: the refund did not reopen the window"
    );
}

/// C-6 — `hold()` wrote `on_time: 0` unconditionally, so a service that calls `hold` before each
/// wake window reset its own meter: `wake = "on", max = "5m"` became "on forever". The meter
/// belongs to the grant, not to the call.
#[test]
fn re_taking_a_hold_cannot_reset_the_on_time_meter() {
    let mut l = WakeLedger::new(&["adsp"]);
    assert!(l.hold("adsp", Some(1), secs(60), secs(5)));
    assert!(l.spend("adsp", secs(5)), "the whole 5 s budget goes on the first window");
    assert!(
        !l.hold("adsp", Some(1), secs(60), secs(5)),
        "C-6: re-taking the hold for the same grant cannot mint a fresh budget"
    );
    assert!(
        !l.spend("adsp", secs(1)),
        "and the meter stays spent after the refused re-entry"
    );

    // The carry-over, not just the refusal: a partially spent grant keeps what it already spent.
    l.release("adsp");
    assert!(l.hold("adsp", Some(2), secs(60), secs(5)));
    assert!(l.spend("adsp", secs(3)));
    assert!(
        l.hold("adsp", Some(2), secs(120), secs(5)),
        "a live grant with budget left may extend its own deadline"
    );
    assert_eq!(
        l.spent_by_grant()
            .into_iter()
            .find(|(g, _)| *g == 2)
            .map(|(_, t)| t),
        Some(secs(3)),
        "C-6: the meter carries over instead of being reset to zero"
    );
    assert!(
        !l.spend("adsp", secs(3)),
        "only the 2 s that is actually left may be spent, not another 5 s"
    );
    assert!(
        l.wake_source("adsp").is_none(),
        "and the overspent window closes instead of being re-armed with a fresh meter"
    );
}

/// C-7 — `Hold.deadline` was stored, documented as absolute-monotonic, and *never compared to
/// anything*: `sleep(dt)` only looked at on-time, so a hold whose deadline passed while the SoC
/// slept stayed `Fm` forever. R7's "power by capability lifetime" is only real if the deadline
/// closes the window by itself.
#[test]
fn a_wake_window_dies_at_its_deadline_not_only_at_its_budget() {
    let mut l = WakeLedger::new(&["apps"]);
    assert!(
        l.hold("apps", Some(1), secs(10), secs(60)),
        "a 10 s deadline with 60 s of on-time still available"
    );
    assert!(l.spend("apps", secs(1)), "the window is open, so the first second is chargeable");
    assert_eq!(l.level_of("apps"), PowerState::Fm);

    // 11 s of sleep: the *deadline* is at 10 s and the budget is nowhere near spent.
    l.sleep(secs(11));
    assert_eq!(
        l.level_of("apps"),
        PowerState::Lp,
        "C-7: the wake window died at its deadline, not at its on-time budget"
    );
    assert_eq!(
        l.wake_source("apps"),
        None,
        "R12: and nothing may be kept powered past the lifetime of the grant that paid for it"
    );
    assert!(
        !l.spend("apps", secs(1)),
        "the closed window charges nothing, even though 59 s of on-time is still unspent"
    );

    // The ledger keeps a monotonic clock, so a deadline that is already behind cannot open a
    // window either — a wall-clock-style "time since I last looked" is exactly what R7 forbids.
    let mut late = WakeLedger::new(&["apps"]);
    late.sleep(secs(100));
    assert!(
        !late.hold("apps", Some(2), secs(10), secs(60)),
        "C-7: a deadline already in the past is refused, not honoured from a frozen clock"
    );
}

/// C-8 — the power ledger's arithmetic was unchecked, so in `--release` (where `+` wraps) the
/// ceiling tests could be stepped over: `on_time + dur` wrapped negative and an overspend was
/// *accepted*, and the per-grant bill could wrap negative too.
#[test]
fn power_arithmetic_cannot_wrap_past_its_ceiling() {
    // The ceiling: an overspend has to be refused, not wrapped into affordability.
    let mut l = WakeLedger::new(&["adsp"]);
    assert!(l.hold("adsp", Some(1), secs(600), secs(600)));
    assert!(l.spend("adsp", secs(599)), "one second short of the ceiling");
    assert!(
        !l.spend("adsp", i64::MAX),
        "C-8: `on_time + dur` must not wrap past the ceiling — in release this *accepted* the overspend"
    );

    // The same wrap in `sleep`: a huge `dt` must close the window, not re-open it.
    let mut s = WakeLedger::new(&["apps"]);
    assert!(s.hold("apps", Some(3), i64::MAX, i64::MAX));
    assert!(s.spend("apps", i64::MAX / 2 + 1));
    s.sleep(i64::MAX / 2 + 1);
    assert_eq!(
        s.level_of("apps"),
        PowerState::Lp,
        "C-8: a sleep cannot wrap the on-time check back inside the budget"
    );

    // And the bill: two domains on one grant must saturate, never report a negative spend.
    let mut pair = WakeLedger::new(&["adsp", "codec"]);
    assert!(pair.hold("adsp", Some(9), secs(600), i64::MAX));
    assert!(pair.hold("codec", Some(9), secs(600), i64::MAX));
    assert!(pair.spend("adsp", i64::MAX / 2 + 1));
    assert!(pair.spend("codec", i64::MAX / 2 + 1));
    let bill = pair.spent_by_grant();
    assert_eq!(bill.len(), 1, "R12: both domains are one wake budget, so they merge");
    assert_eq!(
        bill[0].1,
        i64::MAX,
        "C-8: the per-grant bill saturates instead of wrapping negative in release"
    );
}

/// C-9 — a deadline is a *lifetime*, not a budget. `budget_covers` only demanded a power-bounding
/// budget when the grant had **no** deadline, so a hand-built `model.infer` grant with
/// `Budget::Unlimited` and any deadline was unlimited inference until the clock ran out. `mint`
/// refuses to write such a grant, but the suite's own rule is that the runtime check fails closed
/// on a bypassed mint — and that coverage existed only for `audio.capture`.
#[test]
fn a_deadline_is_not_a_budget_for_a_power_spending_kind() {
    let mut g = grant(
        "model.infer",
        "m",
        Scope::Model { model: "x".into() },
        Budget::Unlimited,
    );
    g.deadline = Some(secs(60));
    assert!(
        !g.budget_covers(&Want::Use, 0),
        "C-9: `Unlimited` plus a deadline is unlimited inference until the clock runs out — R4's \
         \"never unlimited\" is about the budget, not about the grant's lifetime"
    );

    // The same grant through the decision point, not only through the predicate: a hand-built or
    // bypassed-mint grant table must refuse it at check time.
    let mut table = GrantTable::new();
    table.issue(g);
    assert_eq!(
        table.check(&Request::new(
            "m",
            "model.infer",
            Scope::Model { model: "x".into() },
            Want::Use,
            0,
        )),
        Decision::Denied(Denied::BudgetSpent),
        "C-9: the refusal is a spent-budget refusal, and R5 requires it to name that"
    );

    // And the bounded form of the same authority still works: `Uses` is what `mint` writes here.
    let bounded = grant(
        "model.infer",
        "m",
        Scope::Model { model: "x".into() },
        Budget::Uses(3),
    );
    assert!(
        bounded.budget_covers(&Want::Use, 0),
        "the fix refuses *unbounded* power, not inference itself"
    );
}

// ── Luna merge review 2026-10-24 (LUNA-MERGE-REVIEW-20261024-2102.md) ──────────────────────

/// F-1 — the C-6 meter only lived as long as the `Hold` it was carried from. A refusal *clears*
/// the hold, so the next call with the same grant took the fresh-hold branch and started from
/// zero: simply repeating a refused hold defeated the per-grant ceiling. The review's Codex
/// reproduction (`caps-repeat-hold-repro.rs`, exit 101) is hold → spend to the ceiling → refused
/// re-hold → **accepted** re-hold.
#[test]
fn a_refused_hold_cannot_be_retried_into_a_fresh_budget() {
    let mut l = WakeLedger::new(&["adsp"]);
    assert!(l.hold("adsp", Some(1), secs(60), secs(5)));
    assert!(l.spend("adsp", secs(5)), "the whole 5 s budget goes on the first window");
    assert!(
        !l.hold("adsp", Some(1), secs(60), secs(5)),
        "the first re-hold is refused (C-6)"
    );
    assert!(
        !l.hold("adsp", Some(1), secs(60), secs(5)),
        "F-1: and the *second* re-hold is refused too — the meter has to outlive the hold it closed"
    );
    assert!(
        !l.spend("adsp", secs(1)),
        "F-1: and no further spend can succeed under that grant"
    );
    for attempt in 0..8 {
        assert!(
            !l.hold("adsp", Some(1), secs(60), secs(5)),
            "F-1: attempt {attempt} is still refused"
        );
        assert!(
            !l.spend("adsp", secs(1)),
            "F-1: and attempt {attempt} still cannot spend"
        );
    }

    // The ceiling is per grant, and a release is the way back — not another attempt.
    assert!(
        l.hold("adsp", Some(2), secs(60), secs(5)),
        "a different grant is a different authority"
    );
    l.release("adsp");
    assert!(
        l.hold("adsp", Some(1), secs(60), secs(5)),
        "release — what the caller does when the grant dies — is the only reset (R7)"
    );
}
/// F-2 — exclusive arbitration keyed on the *service name*, so two generations of one service
/// could each run their own live exclusive grant on the same device. The grant model makes
/// `(service, instance)` the holder identity (B9: a rollback revokes the old generation's grants
/// precisely because the generation is part of the identity), and exclusivity means one holder.
#[test]
fn one_service_two_generations_are_still_one_holder() {
    let mic = || Scope::Device {
        device: "wcd9340.mic0".into(),
    };
    let mut table = GrantTable::new();

    let mut old_gen = grant(
        "audio.capture",
        "assistant",
        mic(),
        Budget::OnTime {
            total: secs(60),
            window: None,
            max_open: Some(secs(60)),
        },
    );
    old_gen.exclusive = true;
    old_gen.instance = 0;
    table.issue(old_gen);

    // The *same* service name, a different generation: a restart, not a second service.
    let mut new_gen = grant(
        "audio.capture",
        "assistant",
        mic(),
        Budget::OnTime {
            total: secs(60),
            window: None,
            max_open: Some(secs(60)),
        },
    );
    new_gen.exclusive = true;
    new_gen.instance = 1;
    table.issue(new_gen);

    assert!(
        table
            .check(&minted("assistant", 0, "audio.capture", mic(), Want::Stream { dur: secs(1) }, 0))
            .allowed(),
        "generation 0 holds the microphone"
    );
    let restart = table.check(&minted(
        "assistant",
        1,
        "audio.capture",
        mic(),
        Want::Stream { dur: secs(1) },
        0,
    ));
    assert_eq!(
        restart,
        Decision::Denied(Denied::Contended),
        "F-2: one microphone, one holder — a *different generation* of the same service is a \
         different holder, so its own live exclusive grant does not put it on the mic as well"
    );

    // The handover path stays open: revocation, not co-holding.
    table.revoke(1);
    assert!(
        table
            .check(&minted("assistant", 1, "audio.capture", mic(), Want::Stream { dur: secs(1) }, 0))
            .allowed(),
        "and the new generation gets the device once the old generation's grant is revoked (B9)"
    );
}

// ── R8 lifecycle: a floor throttle is not a grant retirement ─────────────────────────────

/// R8/R4 — a *temporary* power-off by the safety floor must not be read as the grant retiring.
/// R8 lets `powermand` throttle *below* a routine's budget (cut the rail once the SoC crosses a
/// thermal ceiling), and while the rail is down the grant behind it is still **live**. The ledger
/// used to offer only `release` for this — the call its own doc-comment assigned to the floors —
/// and `release` also wiped the domain's on-time meters. So a thermal dip handed the still-live
/// grant a whole budget back: `max = "5m"` stopped being a ceiling, which is precisely the
/// "hold a resource clock unbounded" state that R8 exists to prevent. `throttle` is the power-off
/// that does not retire the grant.
#[test]
fn a_throttled_rail_cannot_be_re_armed_with_a_fresh_budget() {
    let mut l = WakeLedger::new(&["adsp"]);
    // The two halves of `wake = "on", max = "5m"`: a deadline, and a 5 s on-time budget.
    assert!(l.hold("adsp", Some(1), secs(60), secs(5)), "a 5 s on-time budget");
    assert!(l.spend("adsp", secs(4)), "4 s of it goes on the first wake window");

    // The thermal dip: the floor cuts the rail *below* the routine's budget. The grant behind the
    // rail never died, so this is a throttled window, not a retired grant.
    assert!(l.throttle("adsp"), "R8: the floor can cut the rail, and the rail really goes down");
    assert_eq!(l.level_of("adsp"), PowerState::Lp, "idle, not off: the wake input stays armed");

    // The recovery: the SoC has cooled, the *same* grant is still live, so powermand re-powers the
    // rail for it. What must NOT come back with the power is the on-time budget.
    assert!(
        l.hold("adsp", Some(1), secs(60), secs(5)),
        "R8: a throttle closes a *window*, it does not retire the *grant* — re-arming has to stay \
         possible or the floor cannot throttle at all"
    );
    assert_eq!(
        l.spent_by_grant()
            .into_iter()
            .find(|(g, _)| *g == 1)
            .map(|(_, t)| t),
        Some(secs(4)),
        "R12: the re-armed window must be billed for the 4 s this grant already spent, not \
         start clean — '0 s spent' after 4 s of on-time is the bill lying"
    );
    assert!(
        !l.spend("adsp", secs(2)),
        "R4/R8: only the 1 s that is actually left may be spent — a thermal dip cannot hand a \
         live grant a second 5 s budget"
    );

    // And the same door, walked through twice: a floor that throttles on every dip would refill
    // the budget once per dip, which is the unbounded wake window in the shape of a finite one.
    let mut duty = WakeLedger::new(&["adsp"]);
    assert!(duty.hold("adsp", Some(7), secs(600), secs(5)));
    let mut windows = 0;
    while windows < 40 {
        if !duty.spend("adsp", secs(1)) {
            break; // the budget closed, which is the behaviour a ceiling has to have
        }
        duty.throttle("adsp"); // the floor cuts the rail between windows
        if !duty.hold("adsp", Some(7), secs(600), secs(5)) {
            break; // …and the re-arm is refused once the budget is gone
        }
        windows += 1;
    }
    assert!(
        windows < 40,
        "R4: throttling between windows must not refill the budget ({windows} windows on a 5 s \
         budget means the ceiling closed, not that it held)",
    );
}

/// R7 — the other half, and the reason the fix cannot simply refuse to reset: when the grant
/// behind the rail really *is* gone (expired, revoked, spent, or the service stopped), releasing
/// the domain **is** the reset, and a new grant on that domain is a new authority that gets its
/// own budget. `throttle` and `release` are different facts, and only one of them is a reset.
#[test]
fn retiring_a_dead_grant_is_the_only_way_back_to_a_fresh_budget() {
    let mut l = WakeLedger::new(&["adsp"]);
    assert!(l.hold("adsp", Some(1), secs(60), secs(5)));
    assert!(l.spend("adsp", secs(4)));
    assert!(l.throttle("adsp"), "a throttle pauses the window, meter and all");
    assert!(
        !l.spend("adsp", secs(2)),
        "and while the rail is down the 1 s that is left is all the grant still has"
    );

    // The grant now *dies*: its deadline is what retires the domain, and that is a reset.
    assert!(l.release("adsp"), "R7: the grant behind the rail is gone, so the domain is released");
    assert!(
        l.hold("adsp", Some(2), secs(60), secs(5)),
        "R7: a new grant on the released domain is a new authority, with its own budget"
    );
    assert!(
        l.spend("adsp", secs(4)),
        "which the new grant may really spend: the meter restarts at 0, and that is correct here"
    );
    assert_eq!(
        l.spent_by_grant()
            .into_iter()
            .find(|(g, _)| *g == 2)
            .map(|(_, t)| t),
        Some(secs(4)),
        "R12: and the bill charges the 4 s to the new grant, not to the one that already died"
    );
}

/// R12 — the bill has to stay a *bill*, not a snapshot of what happens to be powered while
/// someone looks. `spent_by_grant` read only the live `Hold`, and a `throttle` — like a `sleep` or
/// a refusal — closes the hold that carried the meter while leaving the **grant live**. So the
/// most expensive grant on the phone dropped off the bill the moment the floor cut its rail: 4 s
/// of a 5 s budget spent, thermal dip, and the bill said "0 s" *while the grant still owed the 4*.
/// Pinned at the point where that was wrong — between the throttle and the re-arm, with no live
/// hold anywhere on the domain — and again after the re-arm, which is where double-counting would
/// creep in if the bill were rebuilt by *adding* a second source instead of moving to the meter.
#[test]
fn the_power_bill_survives_the_throttle_that_closed_its_window() {
    let mut l = WakeLedger::new(&["adsp"]);
    assert!(l.hold("adsp", Some(1), secs(60), secs(5)));
    assert!(l.spend("adsp", secs(4)), "4 s of a 5 s budget, spent while the rail was up");

    // The thermal dip: the rail goes down, the grant does not.
    assert!(l.throttle("adsp"), "R8: the floor cuts the rail");
    assert_eq!(
        l.wake_source("adsp"),
        None,
        "setup: nothing holds the rail up any more — which is exactly what used to erase the bill"
    );
    assert_eq!(
        l.spent_by_grant(),
        vec![(1, secs(4))],
        "R12: with no live hold to read, the bill must fall back on the durable meter — a grant \
         that is still live and spent 4 s owes that 4 s whether or not it is spending this second"
    );

    // The re-arm, checked twice for a reason: if the bill ever sums `live + recorded` rather than
    // reading the one meter, this is where 4 s quietly becomes 8 s.
    assert!(l.hold("adsp", Some(1), secs(60), secs(5)), "the cooled rail comes back up");
    assert_eq!(
        l.spent_by_grant(),
        vec![(1, secs(4))],
        "R12: and the re-armed window is the SAME 4 s, not a second one"
    );
    assert!(!l.spend("adsp", secs(2)), "R4: which is also why only the 1 s left is spendable");
}

/// R12 — a routine that needs DSP *and* codec has **one** wake budget, so it gets one bill line.
/// Checked while both rails are up (the aggregation) and after both are throttled (the half that
/// used to vanish): the pair of assertions that stops a fix for one hole from opening the other,
/// since merging per domain and doubling the total would each look fine on one side alone.
#[test]
fn one_grant_on_two_rails_is_one_bill_line_even_while_throttled() {
    let mut l = WakeLedger::new(&["adsp", "codec"]);
    assert!(l.hold("adsp", Some(7), secs(600), secs(10)));
    assert!(l.hold("codec", Some(7), secs(600), secs(10)), "one grant, two rails");
    assert!(l.spend("adsp", secs(2)));
    assert!(l.spend("codec", secs(3)));
    assert_eq!(
        l.spent_by_grant(),
        vec![(7, secs(5))],
        "R12: one line for the pair, and it is the *sum* (2 + 3) — not two lines, not one rail"
    );

    // Now cut the power on both rails. The grant is still live; the 5 s is still what it cost.
    assert!(l.throttle("adsp"));
    assert!(l.throttle("codec"));
    assert_eq!(
        l.wake_source("adsp"),
        None,
        "setup: neither rail has a live hold, so a bill that reads holds would come back empty"
    );
    assert_eq!(
        l.spent_by_grant(),
        vec![(7, secs(5))],
        "R12: a throttled pair still owes its on-time — the bill is not a power gauge"
    );

    // Re-armed, that one line has to survive with the same total.
    assert!(l.hold("adsp", Some(7), secs(600), secs(10)));
    assert!(l.hold("codec", Some(7), secs(600), secs(10)));
    assert_eq!(
        l.spent_by_grant(),
        vec![(7, secs(5))],
        "R12: re-arming both rails must not bill the 5 s twice — two lines or 10 s both fail R12"
    );
}

/// R4/R12 — the ceiling and the bill are one piece of bookkeeping seen from two sides: the
/// ceiling refuses the *next* window, the bill remembers the ones that already happened. A floor
/// that throttles between windows must therefore neither refill the budget nor blank the bill,
/// which is why both are asserted per cycle: a bill that resets on each dip looks healthy on
/// every line while telling an outright lie about the total.
#[test]
fn throttling_between_windows_neither_refills_the_budget_nor_blanks_the_bill() {
    let mut l = WakeLedger::new(&["adsp"]);
    assert!(l.hold("adsp", Some(3), secs(600), secs(5)), "a 5 s on-time budget");

    // 40 one-second windows would be 40 s of on-time against a 5 s ceiling.
    let mut bill_at_window = Vec::new();
    for _ in 0..40 {
        if !l.spend("adsp", secs(1)) {
            break;
        }
        bill_at_window.push(l.spent_by_grant());
        l.throttle("adsp");
        if !l.hold("adsp", Some(3), secs(600), secs(5)) {
            break;
        }
        assert_eq!(
            l.spent_by_grant(),
            *bill_at_window.last().unwrap(),
            "the re-arm must not re-bill on-time already charged to this grant"
        );
    }

    assert_eq!(
        bill_at_window.len(),
        5,
        "R4: five 1 s windows fit a 5 s budget and the sixth must be refused (a budget that \
         survives 40 throttles is not a ceiling)"
    );
    assert_eq!(
        l.spent_by_grant(),
        vec![(3, secs(5))],
        "R12: and after the last refusal the bill still says 5 s, not 0 s"
    );
}

/// R7/R12 — the boundary the three tests above lean on. Retiring a grant *is* still allowed to
/// clear the ledger's records, because the next grant on that domain is a new authority: that is
/// the documented semantics, and a bill that can never be cleared is a leak, not a ledger. Pinned
/// so "the meter survives throttling" cannot be misread as "the meter survives forever".
#[test]
fn retiring_a_grant_clears_its_bill_line_and_a_new_grant_starts_clean() {
    let mut l = WakeLedger::new(&["adsp"]);
    assert!(l.hold("adsp", Some(1), secs(60), secs(5)));
    assert!(l.spend("adsp", secs(4)));
    assert!(l.throttle("adsp"), "pause first: the window closes, the meter stays");
    assert_eq!(l.spent_by_grant(), vec![(1, secs(4))], "and while paused the 4 s is still owed");

    assert!(l.release("adsp"), "R7: now retire the domain — the grant behind it is gone");
    assert!(
        l.spent_by_grant().is_empty(),
        "R7: a retired grant leaves no bill line behind, or the bill is a growing leak"
    );
    assert!(l.hold("adsp", Some(2), secs(60), secs(5)));
    assert!(l.spend("adsp", secs(4)), "so the new grant really does get a whole budget");
    assert_eq!(
        l.spent_by_grant(),
        vec![(2, secs(4))],
        "R12: and it is billed to the new grant alone — the dead grant's 4 s is not charged twice"
    );
}
