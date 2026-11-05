// Kimono (Track A2) — the pure half of the open-kimono view.
//
// `apply(view, event)` folds one trace-events journal record into the view
// model and reports what it did. It touches no DOM and no network, so the
// same code runs in the phone browser and in the headless node test.
//
// `apply` returns { action, item } where action is:
//   'created'  a new visible timeline item
//   'updated'  an existing item changed (streaming text, span close, …)
//   'state'    a panel changed (mic meter, power chip, service chip)
//   'unhandled' the record carries a vocabulary this build has no case for
// 'unhandled' is never silent: it appends a visible item and a counter.

export function newView() {
  return {
    seq: -1,
    run: null, // { threadId, runId, state, step }
    banner: [], // [{ level, text }]
    mic: { live: false, level: 0, device: null },
    followUp: { active: false, session: null }, // CV1 follow-up window (listener-reported)
    services: [], // [{ name, generation, state, reason, trigger }]
    power: {}, // component -> { state, reason }
    wakeLocks: [], // [{ holder, deadline_us }]
    grants: [], // [{ service, capability, scope }]
    spans: {}, // span_id -> { name, status, attrs, ts_us }
    tools: {}, // toolCallId -> { name, args, result, status }
    messages: {}, // messageId -> { role, text, done }
    reasoning: {}, // messageId -> { text, done }
    items: [], // timeline items
    modules: {}, // module id -> { manifest, inputs } (agent-emitted, journal-only)
    moduleOrder: [], // module ids in first-emission order; latest wins per id
    unhandled: 0,
    dropped: 0,
    lastRequestText: null, // text of the latest RUN_STARTED input (the request a gate blocked)
    pendingConfirm: null, // { label, tool, reason, text } while an ask_to_commit card is open
  };
}

const KNOWN_PANELS = new Set([
  'run', 'step', 'message', 'reasoning', 'tool', 'span', 'service', 'power', 'grant',
  'mic', 'stt', 'state', 'activity', 'subagent', 'raw', 'banner', 'interrupt', 'confirm',
]);

export function knownPanels() {
  return [...KNOWN_PANELS];
}

// Agent-emitted modules in first-emission order, latest per id.
export function moduleEntries(view) {
  return (view.moduleOrder || []).map((id) => view.modules[id]).filter(Boolean);
}

function item(view, panel, kind, text, meta) {
  const it = { id: `${panel}-${view.items.length}`, seq: view.seq, panel, kind, text, meta: meta || {} };
  view.items.push(it);
  return it;
}

function banner(view, level, text) {
  const b = { level, text };
  view.banner.push(b);
  if (view.banner.length > 6) view.banner.shift();
  return b;
}

function fmtUs(us) {
  if (us === null || us === undefined) return '';
  return us >= 1000000 ? `${(us / 1000000).toFixed(2)} s` : `${Math.round(us / 1000)} ms`;
}

function shortJson(v) {
  if (v === undefined || v === null) return '';
  const s = typeof v === 'string' ? v : JSON.stringify(v);
  return s.length > 400 ? `${s.slice(0, 400)}…` : s;
}

// A CUSTOM value arrives as an object on the wire; tolerate a JSON string
// so a replay that stringified it still yields the gate's label.
function customValue(v) {
  if (v && typeof v === 'object') return v;
  if (typeof v === 'string') {
    try { const p = JSON.parse(v); return p && typeof p === 'object' ? p : {}; } catch { return {}; }
  }
  return {};
}

// The exact body a Confirm click re-POSTs: the SAME request text, plus
// confirm naming exactly the blocked label (W1 wire contract). Nothing
// else — no tool name, no extra keys. Null when there is nothing lawful
// to send (no label, or the original text was never seen).
export function confirmRequestBody(pending) {
  if (!pending || typeof pending.label !== 'string' || pending.label === '') return null;
  if (typeof pending.text !== 'string' || pending.text === '') return null;
  return { text: pending.text, confirm: pending.label };
}

// POST the confirm body through an injected fetch (the browser passes the
// real one; the headless test passes a spy and reads the exact bytes).
export async function postConfirm(fetchImpl, pending) {
  const body = confirmRequestBody(pending);
  if (!body) return null;
  return fetchImpl('/request', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
}

// Cancel dismisses the card and nothing more: it takes no fetch, so it
// cannot post. Returns the dismissed pending (or null) for the caller.
export function cancelConfirm(view) {
  const had = view.pendingConfirm || null;
  view.pendingConfirm = null;
  return had;
}

// ——— AG-UI ———
function applyAgUi(view, ev) {
  switch (ev.type) {
    case 'RUN_STARTED':
      view.run = { threadId: ev.threadId, runId: ev.runId, state: 'running', step: null };
      if (ev.input && typeof ev.input.text === 'string' && ev.input.text !== '') view.lastRequestText = ev.input.text;
      return { action: 'created', item: item(view, 'run', 'run-started', `run ${ev.runId}`, { threadId: ev.threadId, input: shortJson(ev.input) }) };
    case 'RUN_FINISHED': {
      if (view.run) view.run.state = 'finished';
      const outcome = ev.outcome && ev.outcome.type;
      const used = (ev.usage || []).map((u) => `${u.input_tokens ?? '?'}→${u.output_tokens ?? '?'} tok`).join(', ');
      const it = item(view, 'run', 'run-finished', `run ${ev.runId} finished${outcome ? ` (${outcome})` : ''}`, { usage: used, result: shortJson(ev.result) });
      if (outcome === 'interrupt') {
        for (const i of (ev.outcome.interrupts || [])) {
          banner(view, 'info', i.message || `agent is waiting: ${i.reason}`);
          item(view, 'interrupt', 'interrupt', i.message || `waiting: ${i.reason}`, { id: i.id, schema: shortJson(i.response_schema) });
        }
      }
      return { action: 'created', item: it };
    }
    case 'RUN_ERROR':
      if (view.run) view.run.state = 'error';
      banner(view, 'error', ev.message || 'run failed');
      return { action: 'created', item: item(view, 'run', 'run-error', `run error${ev.code ? ` [${ev.code}]` : ''}: ${ev.message || ''}`) };
    case 'STEP_STARTED':
      if (view.run) view.run.step = ev.stepName;
      return { action: 'created', item: item(view, 'step', 'step-started', `step: ${ev.stepName}`) };
    case 'STEP_FINISHED':
      return { action: 'created', item: item(view, 'step', 'step-finished', `step done: ${ev.stepName}`) };

    case 'TEXT_MESSAGE_START':
    case 'TEXT_MESSAGE_CHUNK': {
      const m = (view.messages[ev.messageId] ||= { role: ev.role || 'assistant', text: '', done: false });
      if (ev.role) m.role = ev.role;
      if (typeof ev.delta === 'string') m.text += ev.delta;
      return { action: 'created', item: item(view, 'message', ev.type, '', { messageId: ev.messageId, role: m.role, text: m.text }) };
    }
    case 'TEXT_MESSAGE_CONTENT': {
      const m = (view.messages[ev.messageId] ||= { role: 'assistant', text: '', done: false });
      m.text += ev.delta || '';
      const it = item(view, 'message', 'stream', '', { messageId: ev.messageId, role: m.role, text: m.text });
      return { action: 'updated', item: it };
    }
    case 'TEXT_MESSAGE_END': {
      const m = (view.messages[ev.messageId] ||= { role: 'assistant', text: '', done: false });
      m.done = true;
      return { action: 'updated', item: item(view, 'message', 'end', '', { messageId: ev.messageId, role: m.role, text: m.text }) };
    }

    case 'TOOL_CALL_START':
    case 'TOOL_CALL_CHUNK': {
      const id = ev.toolCallId || `chunk-${Object.keys(view.tools).length}`;
      const t = (view.tools[id] ||= { name: ev.toolCallName || '?', args: '', result: null, status: 'running' });
      if (ev.toolCallName) t.name = ev.toolCallName;
      if (typeof ev.delta === 'string') t.args += ev.delta;
      return { action: 'created', item: item(view, 'tool', ev.type, `tool ${t.name}`, { toolCallId: id, args: t.args, status: t.status }) };
    }
    case 'TOOL_CALL_ARGS': {
      const t = (view.tools[ev.toolCallId] ||= { name: '?', args: '', result: null, status: 'running' });
      t.args += ev.delta || '';
      return { action: 'updated', item: item(view, 'tool', 'args', `args ${t.name}`, { toolCallId: ev.toolCallId, args: t.args, status: t.status }) };
    }
    case 'TOOL_CALL_END': {
      const t = view.tools[ev.toolCallId] || { name: '?', args: '', status: 'running' };
      return { action: 'updated', item: item(view, 'tool', 'end', `args complete: ${t.name}`, { toolCallId: ev.toolCallId, args: t.args }) };
    }
    case 'TOOL_CALL_RESULT': {
      const t = (view.tools[ev.toolCallId] ||= { name: '?', args: '', result: null, status: 'running' });
      t.result = ev.content;
      t.status = ev.content && ev.content.ok === false ? 'error' : 'done';
      if (t.status === 'error') banner(view, 'error', `tool ${t.name} failed: ${shortJson(ev.content && ev.content.error)}`);
      return { action: 'updated', item: item(view, 'tool', 'result', `result ${t.name}: ${shortJson(ev.content)}`, { toolCallId: ev.toolCallId, result: shortJson(ev.content), status: t.status }) };
    }

    case 'STATE_SNAPSHOT':
      applyState(view, ev.snapshot, true);
      return { action: 'created', item: item(view, 'state', 'state-snapshot', 'state snapshot', { snapshot: shortJson(ev.snapshot) }) };
    case 'STATE_DELTA':
      applyState(view, ev.delta, false);
      return { action: 'created', item: item(view, 'state', 'state-delta', 'state delta', { delta: shortJson(ev.delta) }) };
    case 'MESSAGES_SNAPSHOT': {
      const list = Array.isArray(ev.messages) ? ev.messages : [];
      for (const m of list) if (m && m.id) view.messages[m.id] = { role: m.role || 'assistant', text: m.content || '', done: true };
      return { action: 'created', item: item(view, 'state', 'messages-snapshot', `messages snapshot (${list.length})`, { count: list.length }) };
    }
    case 'ACTIVITY_SNAPSHOT':
      return { action: 'created', item: item(view, 'activity', 'activity-snapshot', `activity ${ev.activityType}`, { content: shortJson(ev.content) }) };
    case 'ACTIVITY_DELTA':
      return { action: 'created', item: item(view, 'activity', 'activity-delta', `activity patch ${ev.activityType}`, { patch: shortJson(ev.patch) }) };
    case 'RAW':
      return { action: 'created', item: item(view, 'raw', 'raw', `raw event${ev.source ? ` from ${ev.source}` : ''}`, { body: shortJson(ev.event) }) };
    case 'CUSTOM': {
      // CV1: the follow-up window's state, reported by the wakeword
      // listener itself (it owns the microphone). This is the ONLY
      // writer of view.followUp, so the "still listening" indicator is
      // the listener's own report — never an inference from run timing.
      if (ev.name === 'follow_up_window') {
        const v = customValue(ev.value);
        const state = typeof v.state === 'string' ? v.state : '';
        if (state === 'open' || state === 'closed') {
          const session = typeof v.session === 'string' ? v.session : null;
          view.followUp = { active: state === 'open', session };
          return { action: 'state', item: item(view, 'mic', 'follow-up-window', state === 'open' ? 'still listening — follow-up window open' : 'follow-up window closed', { session: session || '', state }) };
        }
      }
      // CV1: a run's session context — what conversation the brain was
      // given. A state row: visible in the timeline, never a card.
      if (ev.name === 'session_context') {
        const v = customValue(ev.value);
        const turns = typeof v.prior_turns === 'number' ? v.prior_turns : 0;
        const label = v.fresh ? 'new session' : `session continues (${turns} earlier turn${turns === 1 ? '' : 's'})`;
        return { action: 'created', item: item(view, 'state', 'session-context', `${label}${v.follow_up ? ' · follow-up turn' : ''}`, { session: v.session || '', fresh: !!v.fresh, followUp: !!v.follow_up }) };
      }
      // Modules: agentd emits a module manifest as data (protocol v1).
      // This is the ONLY writer of view.modules — kimono renders
      // manifests only from journaled module_manifest records, never
      // from a page island or a fetch. Latest per id wins, so a card
      // updates in place as the agent re-emits it.
      if (ev.name === 'module_manifest') {
        const v = customValue(ev.value);
        const manifest = v && typeof v.manifest === 'object' && v.manifest !== null && !Array.isArray(v.manifest) ? v.manifest : v;
        if (manifest && typeof manifest === 'object' && !Array.isArray(manifest)) {
          const id = typeof manifest.id === 'string' && manifest.id !== '' ? manifest.id : 'unknown';
          const inputs = v && typeof v.inputs === 'object' && v.inputs !== null && !Array.isArray(v.inputs) ? v.inputs : undefined;
          if (!view.moduleOrder.includes(id)) view.moduleOrder.push(id);
          view.modules[id] = { manifest, inputs };
          return { action: 'state', item: item(view, 'state', 'module-manifest', `module ${id} updated`, { moduleId: id }) };
        }
      }
      // W6: the broker gate (W1) blocks a commit-class tool call and journals
      // ask_to_commit; that one CUSTOM event becomes a confirm card, every
      // other CUSTOM stays the generic raw row.
      if (ev.name === 'ask_to_commit') {
        const v = customValue(ev.value);
        const label = typeof v.label === 'string' ? v.label : '';
        if (label !== '') {
          const tool = typeof v.tool === 'string' ? v.tool : '';
          const reason = typeof v.reason === 'string' && v.reason !== '' ? v.reason : `\`${label}\` changes the phone and needs your confirmation first.`;
          const pending = { label, tool, reason, text: view.lastRequestText || '' };
          view.pendingConfirm = pending;
          return { action: 'created', item: item(view, 'confirm', 'ask-to-commit', `Confirm ${label}? ${reason}`, { label, tool, reason, requestText: pending.text, confirm: label }) };
        }
      }
      return { action: 'created', item: item(view, 'raw', 'custom', `custom ${ev.name}`, { body: shortJson(ev.value) }) };
    }

    case 'REASONING_START':
      view.reasoning[ev.messageId] = { text: '', done: false };
      return { action: 'created', item: item(view, 'reasoning', 'reasoning-start', 'reasoning', { messageId: ev.messageId, text: '' }) };
    case 'REASONING_MESSAGE_START':
    case 'REASONING_MESSAGE_CONTENT':
    case 'REASONING_MESSAGE_CHUNK': {
      const r = (view.reasoning[ev.messageId] ||= { text: '', done: false });
      r.text += ev.delta || '';
      return { action: 'updated', item: item(view, 'reasoning', 'reasoning-text', '', { messageId: ev.messageId, text: r.text }) };
    }
    case 'REASONING_MESSAGE_END':
    case 'REASONING_END': {
      const r = (view.reasoning[ev.messageId] ||= { text: '', done: false });
      r.done = true;
      return { action: 'updated', item: item(view, 'reasoning', 'reasoning-end', '', { messageId: ev.messageId, text: r.text }) };
    }
    case 'REASONING_ENCRYPTED_VALUE':
      return { action: 'created', item: item(view, 'reasoning', 'reasoning-encrypted', `encrypted ${ev.subtype} value (hidden)`, { entityId: ev.entityId }) };

    case 'SUBAGENT_STARTED':
      return { action: 'created', item: item(view, 'subagent', 'subagent-started', `subagent ${ev.name}`, { subagentRunId: ev.subagentRunId, description: ev.description || '' }) };
    case 'SUBAGENT_FINISHED':
      return { action: 'created', item: item(view, 'subagent', 'subagent-finished', `subagent ${ev.subagentRunId} done`, { result: shortJson(ev.result) }) };
    case 'SUBAGENT_ERROR':
      banner(view, 'error', `subagent ${ev.subagentRunId}: ${ev.message}`);
      return { action: 'created', item: item(view, 'subagent', 'subagent-error', `subagent error${ev.code ? ` [${ev.code}]` : ''}: ${ev.message}`) };
    default:
      return unhandled(view, 'ag_ui', ev.type);
  }
}

// A snapshot replaces services/power; a delta is an RFC-6902-ish array we
// apply shallowly (enough to keep the chips honest without a patch library).
function applyState(view, value, replace) {
  if (!value) return;
  if (replace) {
    if (Array.isArray(value.services)) view.services = value.services.map((s) => ({ ...s }));
    if (value.power) for (const [k, v] of Object.entries(value.power)) view.power[k] = { state: v, reason: 'from state snapshot' };
    return;
  }
  if (!Array.isArray(value)) return;
  for (const op of value) {
    if (!op || typeof op.path !== 'string') continue;
    if (op.path === '/services' && Array.isArray(op.value)) view.services = op.value.map((s) => ({ ...s }));
    else if (op.path.startsWith('/power/')) view.power[op.path.split('/')[2]] = { state: op.value, reason: 'from state delta' };
  }
}

// ——— op6 ———
function applyOp6(view, ev) {
  switch (ev.type) {
    case 'mic_started':
      view.mic = { live: true, level: view.mic.level, device: ev.device || 'microphone' };
      return { action: 'created', item: item(view, 'mic', 'mic-started', `mic ON${ev.device ? ` (${ev.device})` : ''}`, { device: ev.device || '' }) };
    case 'mic_stopped':
      view.mic = { live: false, level: 0, device: view.mic.device };
      return { action: 'created', item: item(view, 'mic', 'mic-stopped', `mic OFF: ${ev.reason}`, { reason: ev.reason }) };
    case 'mic_frame':
      view.mic.level = ev.level;
      return { action: 'state', item: item(view, 'mic', 'mic-frame', `level ${Math.round(ev.level * 100)}%`, { level: ev.level }) };
    case 'stt_partial':
      return { action: 'created', item: item(view, 'stt', 'stt-partial', `hearing: ${ev.text}`, { engine: ev.engine, confidence: ev.confidence }) };
    case 'stt_final':
      return { action: 'created', item: item(view, 'stt', 'stt-final', `heard: ${ev.text}`, { engine: ev.engine, confidence: ev.confidence, latency: fmtUs(ev.latency_us) }) };
    case 'text_cleaned':
      return { action: 'created', item: item(view, 'stt', 'text-cleaned', `cleaned: ${ev.cleaned}`, { raw: ev.raw }) };
    case 'power_change':
      view.power[ev.component] = { state: ev.state, reason: ev.reason };
      return { action: 'created', item: item(view, 'power', 'power-change', `${ev.component} → ${ev.state}: ${ev.reason}`, { component: ev.component, state: ev.state, reason: ev.reason }) };
    case 'wake_lock_acquired':
      view.wakeLocks.push({ holder: ev.holder, deadline_us: ev.deadline_us });
      return { action: 'created', item: item(view, 'power', 'wake-lock', `wake lock: ${ev.holder}`, { deadline_us: ev.deadline_us, max_us: ev.max_us }) };
    case 'wake_lock_released':
      view.wakeLocks = view.wakeLocks.filter((w) => w.holder !== ev.holder);
      return { action: 'created', item: item(view, 'power', 'wake-lock-released', `wake lock released: ${ev.holder}${ev.reason ? ` (${ev.reason})` : ''}`) };
    case 'service_started':
      view.services = view.services.filter((s) => s.name !== ev.service).concat([{ name: ev.service, generation: ev.generation, state: 'running', reason: ev.reason }]);
      return { action: 'created', item: item(view, 'service', 'service-started', `service ${ev.service} (gen ${ev.generation}) started: ${ev.reason}`, { service: ev.service, generation: ev.generation, reason: ev.reason }) };
    case 'service_stopped':
      view.services = view.services.filter((s) => s.name !== ev.service);
      return { action: 'created', item: item(view, 'service', 'service-stopped', `service ${ev.service} (gen ${ev.generation}) stopped: ${ev.reason}`, { service: ev.service, reason: ev.reason }) };
    case 'grant_granted':
      view.grants.push({ service: ev.service, capability: ev.capability, scope: ev.scope || '' });
      return { action: 'created', item: item(view, 'grant', 'grant-granted', `grant ${ev.service} → ${ev.capability}${ev.scope ? ` (${ev.scope})` : ''}`, { service: ev.service, capability: ev.capability }) };
    case 'grant_revoked':
      view.grants = view.grants.filter((g) => !(g.service === ev.service && g.capability === ev.capability));
      return { action: 'created', item: item(view, 'grant', 'grant-revoked', `grant revoked ${ev.service} → ${ev.capability}: ${ev.reason}`, { service: ev.service, capability: ev.capability }) };
    case 'handoff': {
      const text = `${ev.destination} — ${ev.status}: ${ev.reason}`;
      if (ev.status === 'failed') banner(view, 'warn', `hand-off failed: ${ev.reason}`);
      if (/local model|llama\.cpp|on-device/i.test(ev.reason + ev.destination) && ev.status === 'started') banner(view, 'warn', `FALLBACK MODEL IN USE — ${ev.reason}`);
      return { action: 'created', item: item(view, 'banner', 'handoff', text, { destination: ev.destination, status: ev.status, reason: ev.reason }) };
    }
    case 'journal_dropped':
      view.dropped += ev.count;
      banner(view, 'warn', `journal dropped ${ev.count} record(s): ${ev.reason}`);
      return { action: 'created', item: item(view, 'banner', 'journal-dropped', `dropped ${ev.count}: ${ev.reason}`, { count: ev.count, reason: ev.reason }) };
    default:
      return unhandled(view, 'op6', ev.type);
  }
}

// ——— gen_ai ———
function applyGenAi(view, ev) {
  const spanId = view._spanId || '(none)';
  const attrs = ev.attributes || {};
  if (ev.event === 'span_start') {
    view.spans[spanId] = { name: ev.name, status: 'open', attrs, ts_us: view._ts_us };
    return { action: 'created', item: item(view, 'span', 'span-start', `span ${ev.name}`, { spanId, attrs: shortJson(attrs) }) };
  }
  if (ev.event === 'span_end') {
    const open = view.spans[spanId] || { name: ev.name, ts_us: view._ts_us };
    const dur = fmtUs(view._ts_us - open.ts_us);
    view.spans[spanId] = { ...open, name: ev.name, status: ev.status, attrs, ts_us: view._ts_us };
    if (ev.status === 'error') banner(view, 'error', `${ev.name} failed: ${attrs['error.message'] || 'no error message'}`);
    return { action: 'updated', item: item(view, 'span', 'span-end', `span ${ev.name} ${ev.status} in ${dur}`, { spanId, status: ev.status, attrs: shortJson(attrs) }) };
  }
  return unhandled(view, 'gen_ai', ev.event);
}

function unhandled(view, kind, type) {
  view.unhandled += 1;
  banner(view, 'warn', `unhandled ${kind} event type: ${type}`);
  return { action: 'unhandled', item: item(view, 'raw', 'unhandled', `UNHANDLED ${kind}/${type}`, { kind, type }) };
}

// Parse one SSE `data:` payload. The error is named, never swallowed: a
// journal line the phone cannot read must show up in the kimono, not vanish.
export function parseLine(raw) {
  if (typeof raw !== 'string' || raw.trim() === '') return { ok: false, error: 'empty journal line' };
  try {
    return { ok: true, event: JSON.parse(raw) };
  } catch (e) {
    return { ok: false, error: `not JSON: ${e.message}` };
  }
}

function looksLikeHex(s, len) {
  return typeof s === 'string' && s.length === len && /^[0-9a-f]+$/.test(s);
}

export function apply(view, ev) {
  if (!ev || typeof ev !== 'object') return unhandled(view, 'envelope', 'not-an-object');
  if (!ev.payload || typeof ev.payload !== 'object') return unhandled(view, 'envelope', 'missing-payload');
  // Validate the envelope as strictly as the Rust reader (crates/trace-events):
  // an off-schema record must never render as if it were fine.
  if (typeof ev.seq !== 'number' || !Number.isFinite(ev.seq)) return unhandled(view, 'envelope', 'bad-seq');
  if (ev.trace_id !== undefined && ev.trace_id !== null && !looksLikeHex(ev.trace_id, 32)) return unhandled(view, 'envelope', 'bad-trace-id');
  for (const k of ['span_id', 'parent_span_id']) {
    if (ev[k] !== undefined && ev[k] !== null && !looksLikeHex(ev[k], 16)) return unhandled(view, 'envelope', `bad-${k.replace('_', '-')}`);
  }
  view.seq = typeof ev.seq === 'number' ? ev.seq : view.seq;
  view._ts_us = typeof ev.ts_us === 'number' ? ev.ts_us : 0;
  view._spanId = ev.span_id || null;
  switch (ev.payload.kind) {
    case 'ag_ui':
      return applyAgUi(view, ev.payload);
    case 'op6':
      return applyOp6(view, ev.payload);
    case 'gen_ai':
      return applyGenAi(view, ev.payload);
    default:
      return unhandled(view, ev.payload.kind || '(no kind)', ev.payload.type || '(no type)');
  }
}
