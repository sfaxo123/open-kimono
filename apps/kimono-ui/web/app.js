// Kimono (Track A2) — browser glue: the SSE feed, the ask box, the stop buttons.
// All rendering logic lives in render.mjs / dom.mjs so the headless test can
// exercise the same code without a browser.

import { newView, apply, parseLine, confirmRequestBody, cancelConfirm, moduleEntries } from './render.mjs';
import { renderItem, renderBanner, renderServices, renderPower, renderMic, renderRun } from './dom.mjs';
import { renderEmittedModules } from './modules.mjs';

const h = (tag, attrs = {}, children = []) => {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) if (v !== null && v !== undefined) el.setAttribute(k, v);
  for (const c of [].concat(children === null ? [] : children)) {
    if (c === null || c === undefined) continue;
    el.appendChild(typeof c === 'string' ? document.createTextNode(c) : c);
  }
  return el;
};

const view = newView();
const els = {
  banner: document.getElementById('banner'),
  mic: document.getElementById('mic'),
  run: document.getElementById('run'),
  services: document.getElementById('services'),
  power: document.getElementById('power'),
  timeline: document.getElementById('timeline'),
  modules: document.getElementById('modules'),
  seq: document.getElementById('seq'),
  conn: document.getElementById('conn'),
  ask: document.getElementById('ask'),
  asktext: document.getElementById('asktext'),
};

function paint() {
  els.banner.replaceChildren(...renderBanner(h, view.banner));
  els.mic.replaceChildren(renderMic(h, view.mic, view.followUp));
  els.run.replaceChildren(renderRun(h, view.run));
  els.services.replaceChildren(...renderServices(h, view.services, stopService));
  els.power.replaceChildren(...renderPower(h, view.power, view.wakeLocks, view.grants));
  els.modules.replaceChildren(...renderEmittedModules(h, moduleEntries(view)));
  els.seq.textContent = `seq ${view.seq}`;
}

function push(it) {
  els.timeline.appendChild(renderItem(h, it));
  // Keep the phone's memory bounded: the kimono shows the last 500 items.
  while (els.timeline.childElementCount > 500) els.timeline.removeChild(els.timeline.firstChild);
  els.timeline.scrollTop = els.timeline.scrollHeight;
}

function note(level, text) {
  view.banner.push({ level, text });
  if (view.banner.length > 6) view.banner.shift();
  paint();
}

let lastRequestText = '';

function postBody(payload) {
  fetch('/request', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(payload),
  }).then((r) => {
    if (!r.ok) return r.text().then((b) => note('error', `request refused (${r.status}): ${b || 'no detail'}`));
  }).catch((e) => note('error', `request failed: ${e.message}`));
}

function sendRequest(text) {
  const t = (text || '').trim();
  if (!t) return;
  lastRequestText = t;
  view.lastRequestText = t;
  postBody({ text: t });
  els.asktext.value = '';
}

// W6 Confirm: re-POST the SAME request text with confirm = the blocked
// label, exactly {text, confirm} (built by confirmRequestBody, the same
// builder the headless test pins). No other keys ever ride along.
function sendConfirm(label, text) {
  const body = confirmRequestBody({ label, text: text || view.lastRequestText || lastRequestText });
  if (!body) { note('error', 'nothing to confirm: the original request text is unknown'); return; }
  postBody(body);
}

function stopService(name) {
  sendRequest(`stop the ${name} service`);
}

els.ask.addEventListener('submit', (ev) => {
  ev.preventDefault();
  sendRequest(els.asktext.value);
});

// On-the-fly module buttons, delegated like the stop buttons. A button
// never calls a tool: it sends the plain-language request it carries
// through the same sendRequest path as the ask box, so the broker and
// confirm rules apply unchanged.
els.modules.addEventListener('click', (ev) => {
  const btn = ev.target.closest('[data-module-action]');
  if (btn) {
    const text = btn.getAttribute('data-request');
    if (text) sendRequest(text);
  }
});

els.services.addEventListener('click', (ev) => {
  const btn = ev.target.closest('[data-stop]');
  if (btn) stopService(btn.getAttribute('data-stop'));
});

// W6 card actions, delegated like the stop buttons. Confirm dismisses the
// card and re-POSTs; Cancel only dismisses (cancelConfirm posts nothing).
els.timeline.addEventListener('click', (ev) => {
  const confirmBtn = ev.target.closest('[data-confirm]');
  if (confirmBtn) {
    const label = confirmBtn.getAttribute('data-confirm');
    const text = confirmBtn.getAttribute('data-text');
    const card = confirmBtn.closest('.confirm-card');
    if (card) card.remove();
    view.pendingConfirm = null;
    sendConfirm(label, text);
    return;
  }
  const cancelBtn = ev.target.closest('[data-cancel]');
  if (cancelBtn) {
    const card = cancelBtn.closest('.confirm-card');
    if (card) card.remove();
    cancelConfirm(view);
  }
});

// On-the-fly modules (protocol v1): agentd emits manifests as
// journaled module_manifest records and paint() renders them from
// view.modules — the journal is the only source. Before the first
// emission the area shows the honest empty state; no sample data is
// ever substituted. Nothing here fetches module data and no module
// ever calls a tool itself.

// ——— the feed ———
let source = null;
let retryMs = 500;

function ingest(raw) {
  const parsed = parseLine(raw);
  if (!parsed.ok) {
    note('error', `journal line rejected: ${parsed.error}`);
    return;
  }
  const r = apply(view, parsed.event);
  if (r.item) push(r.item);
  paint();
}

function connect() {
  source = new EventSource('/events');
  source.onopen = () => {
    retryMs = 500;
    els.conn.textContent = 'live';
    els.conn.className = 'conn conn-live';
  };
  source.onmessage = (m) => ingest(m.data);
  source.onerror = () => {
    els.conn.textContent = 'reconnecting';
    els.conn.className = 'conn conn-down';
    source.close();
    note('warn', 'event stream dropped — reconnecting');
    setTimeout(connect, retryMs);
    retryMs = Math.min(retryMs * 2, 8000);
  };
}

connect();
paint();
