// CV1 headless test: the follow-up window's indicator is driven by the
// listener's own follow_up_window journal records and by nothing else.
//
//   node --test test/followup_test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';

import { newView, apply } from '../web/render.mjs';
import { renderMic, renderAll } from '../web/dom.mjs';

// A plain-object DOM, same contract as render_test.mjs.
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
  return (node.children || []).map(textOf).join('');
}

let seq = 0;
function custom(name, value) {
  seq += 1;
  return { seq, ts_us: seq * 1000000, service: 'agentd', trace_id: null, span_id: null, payload: { kind: 'ag_ui', type: 'CUSTOM', name, value } };
}

test('follow_up_window open shows still-listening; closed clears it', () => {
  const view = newView();
  assert.equal(view.followUp.active, false);
  assert.match(textOf(renderMic(h, view.mic, view.followUp)), /mic off/);

  const r = apply(view, custom('follow_up_window', { session: 'sess-1', state: 'open' }));
  assert.equal(r.action, 'state');
  assert.equal(view.followUp.active, true);
  assert.equal(view.followUp.session, 'sess-1');
  const chip = renderMic(h, view.mic, view.followUp);
  assert.match(textOf(chip), /still listening/);
  assert.match(chip.attrs.class, /mic-followup/);

  apply(view, custom('follow_up_window', { session: 'sess-1', state: 'closed' }));
  assert.equal(view.followUp.active, false);
  assert.match(textOf(renderMic(h, view.mic, view.followUp)), /mic off/);
});

test('a malformed window state changes nothing', () => {
  const view = newView();
  apply(view, custom('follow_up_window', { session: 'sess-1', state: 'ajar' }));
  assert.equal(view.followUp.active, false);
});

test('session_context renders as a state row, fresh and continuing', () => {
  const view = newView();
  const a = apply(view, custom('session_context', { session: 'sess-1', prior_turns: 0, entities: {}, pending: [], follow_up: false, fresh: true }));
  assert.equal(a.item.kind, 'session-context');
  assert.match(a.item.text, /new session/);
  const b = apply(view, custom('session_context', { session: 'sess-1', prior_turns: 1, entities: { destination: 'San Francisco' }, pending: [], follow_up: true, fresh: false }));
  assert.match(b.item.text, /session continues \(1 earlier turn\)/);
  assert.match(b.item.text, /follow-up turn/);
});

test('renderAll builds with an open follow-up window', () => {
  const view = newView();
  apply(view, custom('follow_up_window', { session: 'sess-9', state: 'open' }));
  const all = renderAll(h, view);
  assert.match(textOf(all.mic), /still listening/);
});
