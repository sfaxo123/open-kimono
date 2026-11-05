// Kimono (Track A2) — the DOM half, written against an injected `h` factory.
//
// The browser passes a document-backed `h`; the headless node test passes a
// plain-object `h`. Same code path, so "it renders in the test" means the
// same markup the phone builds. No frameworks, no CDN — the phone may be
// offline (PLAN A2).

const TAG_LABEL = {
  'run-started': 'RUN', 'run-finished': 'RUN', 'run-error': 'ERROR',
  'step-started': 'STEP', 'step-finished': 'STEP',
  'user': 'YOU', 'assistant': 'AGENT',
  'reasoning-start': 'THINK', 'reasoning-text': 'THINK', 'reasoning-end': 'THINK', 'reasoning-encrypted': 'THINK',
  'tool': 'TOOL', 'args': 'TOOL', 'end': 'TOOL', 'result': 'TOOL',
  'span-start': 'SPAN', 'span-end': 'SPAN',
  'service-started': 'SERVICE', 'service-stopped': 'SERVICE',
  'power-change': 'POWER', 'wake-lock': 'POWER', 'wake-lock-released': 'POWER',
  'grant-granted': 'GRANT', 'grant-revoked': 'GRANT',
  'mic-started': 'MIC', 'mic-stopped': 'MIC', 'mic-frame': 'MIC', 'follow-up-window': 'MIC', 'session-context': 'STATE',
  'stt-partial': 'STT', 'stt-final': 'STT', 'text-cleaned': 'STT',
  'state-snapshot': 'STATE', 'state-delta': 'STATE', 'messages-snapshot': 'STATE', 'module-manifest': 'MODULE',
  'activity-snapshot': 'ACTIVITY', 'activity-delta': 'ACTIVITY',
  'raw': 'RAW', 'custom': 'CUSTOM', 'unhandled': 'UNKNOWN', 'ask-to-commit': 'CONFIRM',
  'handoff': 'HANDOFF', 'journal-dropped': 'JOURNAL', 'interrupt': 'ASK',
  'subagent-started': 'SUBAGENT', 'subagent-finished': 'SUBAGENT', 'subagent-error': 'SUBAGENT',
};

function textOf(it) {
  const m = it.meta || {};
  if (it.text) return it.text;
  if (it.panel === 'message') return m.text || '';
  if (it.panel === 'reasoning') return m.text || '';
  if (it.panel === 'tool' && it.kind === 'args') return m.args || '';
  return '';
}

function metaLines(it) {
  const m = it.meta || {};
  const out = [];
  for (const k of ['args', 'result', 'attrs', 'snapshot', 'delta', 'body', 'content', 'patch', 'raw', 'reason', 'input', 'usage', 'result']) {
    if (m[k]) out.push(`${k}: ${m[k]}`);
  }
  for (const k of ['service', 'capability', 'component', 'state', 'destination', 'status', 'toolCallId', 'spanId', 'messageId', 'engine', 'confidence', 'latency', 'generation', 'deadline_us', 'subagentRunId', 'description', 'entityId', 'schema', 'count', 'threadId', 'device']) {
    if (m[k] !== undefined && m[k] !== null && m[k] !== '') out.push(`${k}: ${m[k]}`);
  }
  return [...new Set(out)];
}

// W6 confirm card: label + reason, a Confirm button carrying exactly
// what app.js needs to re-POST ({text, confirm:label} via data attrs),
// and a Cancel button that only dismisses. Pure markup — no fetch here.
export function renderConfirmCard(h, card) {
  const label = card.label || '';
  const reason = card.reason || '';
  const requestText = card.requestText || card.text || '';
  return h('div', { class: 'item panel-confirm kind-ask-to-commit confirm-card', 'data-seq': String(card.seq ?? ''), 'data-label': label }, [
    h('span', { class: 'tag tag-confirm' }, 'CONFIRM'),
    h('span', { class: 'text' }, `Confirm ${label}? ${reason}`.trim()),
    h('button', { class: 'confirm-btn', type: 'button', 'data-confirm': label, 'data-text': requestText }, 'Confirm'),
    h('button', { class: 'cancel-btn', type: 'button', 'data-cancel': 'true' }, 'Cancel'),
  ]);
}

export function renderItem(h, it) {
  if (it.kind === 'ask-to-commit') {
    const m = it.meta || {};
    return renderConfirmCard(h, { label: m.label, reason: m.reason, requestText: m.requestText, seq: it.seq });
  }
  const tag = TAG_LABEL[it.kind] || (it.panel || 'event').toUpperCase();
  const meta = metaLines(it);
  const children = [h('span', { class: `tag tag-${it.panel}` }, tag), h('span', { class: 'text' }, textOf(it))];
  if (meta.length) {
    children.push(h('details', { class: 'meta' }, [h('summary', {}, 'detail'), h('pre', {}, meta.join('\n'))]));
  }
  return h('div', { class: `item panel-${it.panel} kind-${it.kind}`, 'data-seq': String(it.seq) }, children);
}

export function renderBanner(h, banners) {
  return banners.map((b) => h('div', { class: `banner banner-${b.level}`, role: 'status' }, b.text));
}

export function renderServices(h, services, onStop) {
  if (!services.length) return [h('div', { class: 'chip chip-empty' }, 'no services running')];
  return services.map((s) =>
    h('div', { class: 'chip chip-service', 'data-service': s.name }, [
      h('span', { class: 'chip-name' }, s.name),
      h('span', { class: 'chip-state' }, s.state || 'running'),
      onStop ? h('button', { class: 'chip-stop', type: 'button', 'data-stop': s.name }, 'stop') : null,
    ]),
  );
}

export function renderPower(h, power, wakeLocks, grants) {
  const chips = Object.entries(power).map(([c, v]) => h('div', { class: `chip chip-power state-${v.state}`, 'data-component': c }, [h('span', { class: 'chip-name' }, c), h('span', { class: 'chip-state' }, v.state)]));
  chips.push(h('div', { class: 'chip chip-count' }, `locks ${wakeLocks.length} · grants ${grants.length}`));
  return chips;
}

export function renderMic(h, mic, followUp) {
  // CV1: while the listener's follow-up window is open the chip says so,
  // in so many words. This branch is driven only by view.followUp, which
  // only the listener's own follow_up_window records may set.
  if (followUp && followUp.active) {
    return h('div', { class: 'mic mic-live mic-followup' }, [
      h('span', { class: 'mic-dot' }, '●'),
      h('span', { class: 'mic-label' }, 'still listening · follow-up window — speak without the wake word'),
      h('span', { class: 'mic-meter', 'aria-hidden': 'true' }, [h('span', { class: 'mic-fill', style: 'width:100%' }, '')]),
    ]);
  }
  const pct = Math.max(0, Math.min(1, mic.level)) * 100;
  return h('div', { class: `mic ${mic.live ? 'mic-live' : 'mic-off'}` }, [
    h('span', { class: 'mic-dot' }, mic.live ? '●' : '○'),
    h('span', { class: 'mic-label' }, mic.live ? `mic live${mic.device ? ` · ${mic.device}` : ''}` : 'mic off'),
    h('span', { class: 'mic-meter', 'aria-hidden': 'true' }, [h('span', { class: 'mic-fill', style: `width:${pct.toFixed(0)}%` }, '')]),
  ]);
}

export function renderRun(h, run) {
  if (!run) return h('div', { class: 'run' }, 'idle');
  return h('div', { class: `run run-${run.state}` }, `run ${run.runId} · ${run.state}${run.step ? ` · ${run.step}` : ''}`);
}

// One pass over the whole view: used by the test to prove every panel builds.
export function renderAll(h, view) {
  return {
    banner: renderBanner(h, view.banner),
    mic: renderMic(h, view.mic, view.followUp),
    run: renderRun(h, view.run),
    services: renderServices(h, view.services, () => {}),
    power: renderPower(h, view.power, view.wakeLocks, view.grants),
    timeline: view.items.map((it) => renderItem(h, it)),
  };
}
