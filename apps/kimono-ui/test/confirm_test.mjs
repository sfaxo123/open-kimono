// W6: the kimono page can answer the broker gate (W1).
//
// Since W1, service_create only runs when the request carries
// "confirm": "<label>", and a blocked call journals a CUSTOM
// ask_to_commit event. This test drives the same render.mjs/dom.mjs the
// phone browser runs, from the checked-in fixture, and pins:
//   - the ask_to_commit event renders as a card showing label + reason,
//     with Confirm/Cancel buttons;
//   - Confirm posts exactly {text, confirm: label} — the SAME request
//     text, no other keys — through the injected fetch app.js uses;
//   - negatives: no card for any other CUSTOM event, Cancel posts
//     nothing, and a gate event with no label confirms nothing.
//
//   node --test test/confirm_test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { newView, apply, confirmRequestBody, postConfirm, cancelConfirm } from '../web/render.mjs';
import { renderItem, renderConfirmCard } from '../web/dom.mjs';
import { start } from '../server.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const LABEL = 'service_create svc.demo.water_reminder';
const REQUEST_TEXT = 'check the wifi and tell me the ssid'; // vocab RUN_STARTED input

function h(tag, attrs = {}, children = []) {
  const node = { tag, attrs: attrs || {}, children: [] };
  for (const c of [].concat(children === null ? [] : children)) {
    if (c === null || c === undefined) continue;
    node.children.push(typeof c === 'string' ? { tag: '#text', text: c } : c);
  }
  return node;
}
function textOf(node) {
  if (node.tag === '#text') return node.text;
  return node.children.map(textOf).join(' ');
}
function findAll(node, pred, out = []) {
  if (pred(node)) out.push(node);
  for (const c of node.children || []) findAll(c, pred, out);
  return out;
}
const buttons = (node) => findAll(node, (n) => n.tag === 'button');

function fixtureEvents() {
  const raw = readFileSync(path.join(HERE, '..', 'fixtures', 'vocab_coverage.jsonl'), 'utf8');
  return raw.split('\n').filter((l) => l.trim() !== '').map((l) => JSON.parse(l));
}

// Fold the fixture up to and including the first event matching pred.
function viewThrough(pred) {
  const view = newView();
  let hit = null;
  for (const ev of fixtureEvents()) {
    const r = apply(view, ev);
    if (pred(ev)) { hit = r; break; }
  }
  return { view, hit };
}
const isAsk = (ev) => ev.payload.type === 'CUSTOM' && ev.payload.name === 'ask_to_commit';
const isBattery = (ev) => ev.payload.type === 'CUSTOM' && ev.payload.name === 'op6.battery';

test('card renders from the fixture: label, reason, Confirm and Cancel', () => {
  const { view, hit } = viewThrough(isAsk);
  assert.ok(hit, 'the fixture must carry an ask_to_commit line (W6 fixture line)');
  assert.equal(hit.action, 'created');
  assert.equal(hit.item.kind, 'ask-to-commit');
  assert.equal(hit.item.panel, 'confirm');
  assert.match(hit.item.text, /service_create svc\.demo\.water_reminder/);
  assert.deepEqual(view.pendingConfirm.label, LABEL);
  assert.equal(view.pendingConfirm.tool, 'service_create');
  assert.equal(view.pendingConfirm.text, REQUEST_TEXT, 'the card must remember the SAME request text');

  const node = renderItem(h, hit.item);
  assert.match(String(node.attrs.class), /confirm-card/, 'the item must render as a confirm card');
  const text = textOf(node);
  assert.ok(text.includes(LABEL), `card must show the label, got: ${text}`);
  assert.ok(text.includes('needs your confirmation first'), `card must show the reason, got: ${text}`);
  const btns = buttons(node);
  const confirmBtn = btns.find((b) => b.attrs['data-confirm'] !== undefined);
  const cancelBtn = btns.find((b) => b.attrs['data-cancel'] !== undefined);
  assert.ok(confirmBtn, 'card must have a Confirm button');
  assert.equal(confirmBtn.attrs['data-confirm'], LABEL);
  assert.equal(confirmBtn.attrs['data-text'], REQUEST_TEXT);
  assert.equal(textOf(confirmBtn), 'Confirm');
  assert.ok(cancelBtn, 'card must have a Cancel button');
  assert.equal(textOf(cancelBtn), 'Cancel');

  // The exported card builder renders the same card straight from a pending.
  const direct = renderConfirmCard(h, view.pendingConfirm);
  assert.ok(textOf(direct).includes(LABEL));
  assert.equal(buttons(direct).length, 2);
});

test('Confirm posts exactly {text, confirm: label} and nothing else', async () => {
  const { view } = viewThrough(isAsk);
  assert.deepEqual(confirmRequestBody(view.pendingConfirm), { text: REQUEST_TEXT, confirm: LABEL });

  const calls = [];
  const fakeFetch = async (url, opts) => { calls.push({ url, opts }); return { ok: true, status: 202 }; };
  await postConfirm(fakeFetch, view.pendingConfirm);
  assert.equal(calls.length, 1, 'Confirm must post once');
  assert.equal(calls[0].url, '/request');
  assert.equal(calls[0].opts.method, 'POST');
  const body = JSON.parse(calls[0].opts.body);
  assert.deepEqual(body, { text: REQUEST_TEXT, confirm: LABEL });
  assert.deepEqual(Object.keys(body).sort(), ['confirm', 'text'], 'no key besides text and confirm may ride along');
});

test('negative: no card for other CUSTOM events', () => {
  const { view, hit } = viewThrough(isBattery);
  assert.ok(hit, 'the fixture must carry the other CUSTOM (op6.battery) before the gate event');
  assert.equal(hit.item.kind, 'custom', 'a non-gate CUSTOM stays the generic row');
  assert.equal(view.pendingConfirm, null, 'a non-gate CUSTOM must open no confirm card');
  const node = renderItem(h, hit.item);
  assert.equal(buttons(node).length, 0, 'a non-gate CUSTOM renders no Confirm/Cancel buttons');
  assert.ok(!String(node.attrs.class).includes('confirm-card'));

  // Synthetic sweep: assorted other CUSTOM names never open a card either.
  for (const name of ['op6.battery', 'telemetry', 'ask_to_commit2', '']) {
    const v = newView();
    const r = apply(v, { seq: 0, ts_us: 1, payload: { kind: 'ag_ui', type: 'CUSTOM', name, value: { label: LABEL, reason: 'x' } } });
    assert.equal(r.item.kind, 'custom', `CUSTOM ${JSON.stringify(name)} must not become a card`);
    assert.equal(v.pendingConfirm, null);
  }
});

test('negative: Cancel posts nothing and leaves nothing to confirm', async () => {
  const { view } = viewThrough(isAsk);
  assert.ok(view.pendingConfirm, 'precondition: a card is open');
  const calls = [];
  const fakeFetch = async (url, opts) => { calls.push({ url, opts }); return { ok: true }; };
  const dismissed = cancelConfirm(view);
  assert.equal(dismissed.label, LABEL);
  assert.equal(view.pendingConfirm, null, 'Cancel must close the card');
  assert.equal(confirmRequestBody(view.pendingConfirm), null, 'after Cancel there is nothing lawful to post');
  assert.equal(await postConfirm(fakeFetch, view.pendingConfirm), null, 'posting a cancelled card is a no-op');
  assert.equal(calls.length, 0, 'Cancel must not post anything');
});

test('negative: a gate event without a label, or without request text, confirms nothing', () => {
  const v = newView();
  const r = apply(v, { seq: 0, ts_us: 1, payload: { kind: 'ag_ui', type: 'CUSTOM', name: 'ask_to_commit', value: { tool: 'service_create', reason: 'no label here' } } });
  assert.equal(r.item.kind, 'custom', 'ask_to_commit with no label cannot offer a Confirm');
  assert.equal(v.pendingConfirm, null);
  assert.equal(confirmRequestBody({ label: LABEL, text: '' }), null);
  assert.equal(confirmRequestBody({ label: '', text: REQUEST_TEXT }), null);
  assert.equal(confirmRequestBody(null), null);
});

test('the replay server accepts the Confirm re-POST and refuses a bad confirm by name', async () => {
  const { server, port } = await start({ port: 0, fixture: 'water_reminder' });
  try {
    const base = `http://127.0.0.1:${port}`;
    const ok = await fetch(`${base}/request`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ text: REQUEST_TEXT, confirm: LABEL }) });
    assert.equal(ok.status, 202);
    const ack = await ok.json();
    assert.equal(ack.ok, true);
    assert.equal(ack.confirm, LABEL, 'the server must record exactly what was confirmed');

    const bad = await fetch(`${base}/request`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ text: REQUEST_TEXT, confirm: 42 }) });
    assert.equal(bad.status, 400);
    assert.match((await bad.json()).error, /"confirm" must be a non-empty string/);
  } finally {
    server.close();
  }
});
