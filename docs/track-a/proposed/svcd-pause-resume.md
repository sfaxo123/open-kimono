# Proposed svcd patch: pause / resume (not applied)

Status: proposal only. `crates/svcd` is out of bounds for the wiring
pass, so nothing here is code that runs.

## Why

User routines are svcd services (W2 deleted agentd's separate cron
engine). Stopping a routine works today (`service_stop`), but pausing
one does not: svcd's `ServiceStatus` has exactly two states, `Running`
and `Stopped { reason, violation }`, and there is no pause ingress.
agentd therefore offers no `service_pause` tool, and a model that asks
for one is refused with ``unknown tool `service_pause` `` (proven by
`negative_pause_is_refused_by_name_and_the_routine_keeps_running` in
`apps/agentd/tests/routines_services.rs`).

## What svcd would need

1. A third status, `Paused { reason: String }`, alongside `Running` and
   `Stopped`, serialized the same way (`"status": "paused"`).
2. `svc:pause` / `svc:resume` ingress mirroring `svc:stop`:
   - pause cancels the service's pending timers (like stop does) but
     keeps grants eligible for resume and does **not** run
     `lifecycle_stop` triggers — the service has not ended;
   - resume restores `Running` and re-arms interval/schedule triggers
     from the resume instant (no catch-up burst for missed firings,
     matching the existing `drop_missed` catch-up policy);
   - both journal `service_paused` / `service_resumed`-style op6 records
     with the reason, the way stop journals its reason today.
3. Power and budget accounting treat `Paused` as not running (a paused
   service must not keep wifi or the local model "needed").

## What agentd would add once that lands

- `service_pause` / `service_resume` tools next to `service_stop`, same
  argument shape (`id`, optional `reason`), both free-class under the
  broker (neither deletes user data).
- Tests through `POST /request`: pause keeps the service listed as
  paused and its timer silent; resume re-arms it; pausing an unknown id
  is refused by name.

Until then the honest user-facing answer to "pause it" is: stop is
available, pause is not.
