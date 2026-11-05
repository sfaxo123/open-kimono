// A2 headless render test: every event type in the fixtures must RENDER.
//
// Runs the same render.mjs + dom.mjs the phone browser runs, against a plain
// object DOM. No browser, no network. Fails loudly if a vocabulary type has
// no rendering, if a panel is never exercised, or if a malformed line is
// silently forgiven.
//
//   node --test test/render_test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { newView, apply, parseLine, knownPanels } from '../web/render.mjs';
import { renderItem, renderAll, renderServices } from '../web/dom.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const FIX = path.join(HERE, '..', 'fixtures');

const GOOD = ['water_reminder', 'fallback_failure', 'voice_mic_power', 'vocab_coverage'];

const AGUI_TYPES = ['TEXT_MESSAGE_START', 'TEXT_MESSAGE_CONTENT', 'TEXT_MESSAGE_END', 'TEXT_MESSAGE_CHUNK', 'TOOL_CALL_START', 'TOOL_CALL_ARGS', 'TOOL_CALL_END', 'TOOL_CALL_CHUNK', 'TOOL_CALL_RESULT', 'STATE_SNAPSHOT', 'STATE_DELTA', 'MESSAGES_SNAPSHOT', 'ACTIVITY_SNAPSHOT', 'ACTIVITY_DELTA', 'RAW', 'CUSTOM', 'RUN_STARTED', 'RUN_FINISHED', 'RUN_ERROR', 'STEP_STARTED', 'STEP_FINISHED', 'REASONING_START', 'REASONING_MESSAGE_START', 'REASONING_MESSAGE_CONTENT', 'REASONING_MESSAGE_END', 'REASONING_MESSAGE_CHUNK', 'REASONING_END', 'REASONING_ENCRYPTED_VALUE', 'SUBAGENT_STARTED', 'SUBAGENT_FINISHED', 'SUBAGENT_ERROR'];
const OP6_TYPES = ['journal_dropped', 'mic_started', 'mic_stopped', 'mic_frame', 'stt_partial', 'stt_final', 'text_cleaned', 'power_change', 'wake_lock_acquired', 'wake_lock_released', 'service_started', 'service_stopped', 'grant_granted', 'grant_revoked', 'handoff'];

// ——— a plain-object DOM that records what the renderer actually built ———
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
function classes(node) {
  return String(node.attrs.class || '');
}

function readFixture(name) {
  return readFileSync(path.join(FIX, `${name}.jsonl`), 'utf8').split('\n').filter((l) => l.trim() !== '').map((l) => JSON.parse(l));
}

test('every fixture event renders and nothing is unhandled', () => {
  const seen = new Set();
  const panels = new Set();
  for (const name of GOOD) {
    const view = newView();
    for (const ev of readFixture(name)) {
      const key = `${ev.payload.kind}/${ev.payload.type || ev.payload.event}`;
      seen.add(key);
      const r = apply(view, ev);
      assert.notEqual(r.action, 'unhandled', `${name} seq ${ev.seq}: ${key} has no rendering`);
      assert.ok(r.item, `${name} seq ${ev.seq}: ${key} produced nothing visible`);
      const node = renderItem(h, r.item);
      assert.equal(node.tag, 'div', `${key} did not render an element`);
      assert.match(classes(node), /(^| )item( |$)/, `${key} rendered without the item class`);
      assert.ok(classes(node).includes(`panel-${r.item.panel}`), `${key} rendered outside its panel`);
      const text = textOf(node).trim();
      assert.ok(text.length > 0, `${name} seq ${ev.seq}: ${key} rendered an empty element`);
    }
    assert.equal(view.unhandled, 0, `${name}: the view counted unhandled records`);
    for (const it of view.items) panels.add(it.panel);
  }
  for (const t of AGUI_TYPES) assert.ok(seen.has(`ag_ui/${t}`), `no fixture exercises ag_ui/${t}`);
  for (const t of OP6_TYPES) assert.ok(seen.has(`op6/${t}`), `no fixture exercises op6/${t}`);
  assert.ok(seen.has('gen_ai/span_start') && seen.has('gen_ai/span_end'), 'gen_ai span pair not exercised');

  // Every panel the renderer claims to know must actually be exercised.
  for (const p of knownPanels()) assert.ok(panels.has(p), `panel ${p} is never exercised by any fixture`);
});

test('the whole view renders: panels, chips, mic meter, stop buttons', () => {
  const view = newView();
  let sawServiceChip = false;
  let sawFallbackBanner = false;
  let sawErrorBanner = false;
  for (const name of GOOD) {
    for (const ev of readFixture(name)) {
      apply(view, ev);
      if (view.banner.some((b) => b.text.includes('FALLBACK MODEL IN USE'))) sawFallbackBanner = true;
      if (view.banner.some((b) => b.level === 'error')) sawErrorBanner = true;
      if (view.services.length) {
        const chips = renderServices(h, view.services, () => {});
        if (chips.some((c) => classes(c).includes('chip-service')) && chips.some((c) => textOf(c).includes('stop'))) sawServiceChip = true;
      }
    }
  }
  const built = renderAll(h, view);
  assert.ok(built.timeline.length === view.items.length, 'every timeline item must build a node');
  assert.ok(sawServiceChip, 'a running service must render a chip with a stop button while it runs');
  assert.ok(built.services.some((c) => classes(c).includes('chip-empty')), 'with nothing running the strip must say so');
  assert.ok(built.power.some((c) => classes(c).includes('chip-power')), 'power state must render chips');
  assert.ok(built.banner.length > 0, 'the failure fixtures must produce banners');
  assert.ok(sawFallbackBanner, 'the fallback hand-off must raise a banner');
  assert.ok(sawErrorBanner, 'a failure must raise an error banner');
  assert.ok(textOf(built.mic).includes('mic off'), 'the mic strip must state whether the mic is live');
});

test('negative: an unknown vocabulary type is reported, not forgiven', () => {
  const view = newView();
  const r = apply(view, { seq: 0, ts_us: 1, payload: { kind: 'ag_ui', type: 'TELEPORTED' } });
  assert.equal(r.action, 'unhandled');
  assert.equal(view.unhandled, 1);
  const node = renderItem(h, r.item);
  assert.match(textOf(node), /UNHANDLED ag_ui\/TELEPORTED/);
  assert.ok(view.banner.some((b) => b.level === 'warn' && b.text.includes('TELEPORTED')), 'an unhandled type must raise a banner');
});

test('negative: a record with no payload is rejected by name', () => {
  const view = newView();
  const r = apply(view, { seq: 0, ts_us: 1 });
  assert.equal(r.action, 'unhandled');
  assert.match(r.item.text, /missing-payload/);
});

test('negative: a malformed journal line names its parse error', () => {
  const bad = parseLine('{"seq":0,"payload":{"kind":"op6","type":"mic_st');
  assert.equal(bad.ok, false);
  assert.match(bad.error, /not JSON/);
  assert.equal(parseLine('').ok, false);
  assert.match(parseLine('').error, /empty journal line/);
  assert.equal(parseLine('{"seq":0,"payload":{"kind":"op6","type":"mic_started"}}').ok, true);
});

test('negative: a fixture file that is not schema JSONL is refused by the reader', () => {
  const junk = readdirSync(path.join(FIX, 'bad'));
  assert.ok(junk.length >= 3, 'the bad/ fixtures must exist for this test to mean anything');
  for (const f of junk) {
    const line = readFileSync(path.join(FIX, 'bad', f), 'utf8').trim();
    const parsed = parseLine(line);
    if (f === 'truncated.jsonl') assert.equal(parsed.ok, false, `${f} must not parse`);
    else if (parsed.ok) {
      // Well-formed JSON but not a valid journal record: apply() must refuse it.
      const view = newView();
      const r = apply(view, parsed.event);
      assert.equal(r.action, 'unhandled', `${f} must not render as a normal event`);
    }
  }
});
