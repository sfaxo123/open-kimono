//! What the model is told about the Lego layer, and the two examples it may
//! copy. Both examples are real IR: they are the fixtures under
//! `apps/agentd/fixtures/`, validated and simulated by `svcd` in
//! `tests/service_tools.rs`, so the model is never shown a document the
//! executor would refuse.

/// The rules the validator actually enforces. Written from the error messages
/// `svcd validate` produces, because those are the ones a model hits.
pub const SHAPE_GUIDE: &str = "\
SERVICE IR (the shape of a service you can create with `service_create`):
A ServiceIR is one JSON object. Required top-level keys: ir_version (\"1.0.0\"),
id, version, generation (1), content_hash (\"blake3:<64 hex>\"), title,
description, author {name, provenance}, execution_mode
(ephemeral_component | warm_pooled_component | singleton_service), priority_class
(P0_SAFETY_THERMAL | P1_USER_ABORT_STOP | P2_INTERACTIVE_VOICE_UI |
P3_ACTIVE_FOREGROUND_NAV | P4_BACKGROUND_MONITOR), parameters, triggers,
workflow (the entry actions), actions, deps {schemes, wasm_components,
sub_services}, capabilities, state_schema {namespace, durability,
wal_limit_bytes, retention_ttl_sec, schema, initial_state, migrations},
ui {\"mode\": \"none\" is fine}, power_budget, tests.
- id is `svc.<area>.<name>`, lowercase, dots between parts.
- triggers: each has id, kind, entry_action, and the kind's block. Kinds:
  voice_intent {voice_intent: {domain, intent_name, lang, method, sentences, slots}},
  timer_schedule {timer_schedule: {mode: interval_monotonic | countdown_monotonic |
  oneshot_iso | cron_wall, duration_sec_expr: \"$.state.some_int\", catchup_policy,
  grace_window_sec, wake_from_suspend}},
  `countdown_monotonic` arms once for a duration read from a parameter, e.g.
  \"duration_sec_expr\": \"{{params.minutes * 60}}\"; `interval_monotonic` re-arms
  itself after each firing, so a recurring reminder is an interval.
  sensor_threshold {sensor_threshold: {scheme_path: \"sys:power/battery\", metric:
  \"capacity_pct\", operator: lt|lte|gt|gte|eq, threshold, hysteresis_reset,
  edge_only, poll_or_irq: spmi_comparator}},
  bus_message, ui_action, lifecycle_stop.
  Optional on any trigger: condition_cel, debounce_ms, cooldown_ms.
- actions: each has type (state_mutate | dialog | ui_patch | bus_emit | logic |
  sub_service), description, effect_class (pure | reversible_staged |
  reversible_compensated | committed_irreversible), read_only, deduplicate_calls,
  concurrency (\"serial\"), timeout {idle_ms, total_ms}, and the type's block
  (state_patch: [{op: replace, path: \"/key\", value: \"{{state.key + 1}}\"}],
  dialog: {speech, text, priority}, ui_patch, bus_emit). `next: [{target}]` chains them.
- The rules the validator refuses, so do not trip them:
  * a `committed_irreversible` action needs `idempotency_key_template`;
  * a `dialog` action needs capability `audio.playback.tts` (or
    `audio.playback.alarm`) AND a writable `audio:` scheme in deps.schemes;
  * a `timer_schedule` trigger needs capability `timer.monotonic.schedule` and a
    `duration_sec_expr` that resolves to a positive number;
  * a `sensor_threshold` trigger on `sys:power/battery` needs capability
    `power.battery.read`;
  * `power_budget.wasm_fuel_per_invocation` must be between 1000 and 500000000;
  * capabilities come from this list only: audio.capture, audio.playback.tts,
    audio.playback.alarm, location.gnss.read, storage.maps.readonly,
    state.namespace.readwrite, power.battery.read, power.request_wake,
    timer.monotonic.schedule, ui.surface.render, network.connect.
- tests is REQUIRED and must be non-empty: each test has name, description,
  initial_env {monotonic_ms, wall_iso, battery_pct, charging, network_online,
  location {lat, lon, tz}}, events [{at_monotonic_ms, event_type: advance_clock |
  utterance | sensor_sample | bus_message | ui_action | stop_signal, payload}],
  assertions {final_state_subset, expected_bus_emissions (in order),
  expected_ui_surfaces, budget_assertions {max_memory_mb_used,
  network_bytes_sent, active_schemes_after_completion}}.
  `network_bytes_sent` and `active_schemes_after_completion` must match exactly;
  a standing_trigger_watch capability leaves its scheme active after completion.
- `service_create` runs your tests in the simulator first. If one fails you get
  the failure text back — fix the IR and call `service_create` again.
- On this phone a service's effects (speech, UI, logic) are journaled, not
  performed: a reminder exists as a journal record the kimono view shows. Never
  tell the user you spoke, displayed or sent something.";

pub const WATER_REMINDER: &str = include_str!("../fixtures/svc.demo.water_reminder.json");
pub const BATTERY_LOW_NOTICE: &str = include_str!("../fixtures/svc.demo.battery_low_notice.json");

/// The whole Lego section: rules first, then two working examples to copy.
pub fn section() -> String {
    format!(
        "\n\n{SHAPE_GUIDE}\n\nEXAMPLE 1 — an interval reminder (svc.demo.water_reminder):\n{WATER_REMINDER}\n\nEXAMPLE 2 — a battery-threshold notice (svc.demo.battery_low_notice):\n{BATTERY_LOW_NOTICE}\n"
    )
}
