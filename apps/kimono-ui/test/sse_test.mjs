// A2 contract test: the kimono replay server must speak exactly the SSE
// shape agentd has to speak — one journal record per `data:` line, `id:` =
// the envelope `seq`, `Last-Event-ID` resumes after that seq.
//
//   node --test test/sse_test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';

import { start, listFixtures, loadFixture } from '../server.mjs';

const FIXTURE = 'water_reminder';

async function withServer(fn) {
  const { server, port } = await start({ port: 0, fixture: FIXTURE });
  assert.equal(server.address().address, '127.0.0.1', 'the replay server must bind loopback only');
  try {
    await fn(`http://127.0.0.1:${port}`);
  } finally {
    server.close();
  }
}

// Read SSE frames off a live stream until `want` frames or the abort fires.
async function readFrames(base, { query = '', headers = {}, want = Infinity, ms = 2500 } = {}) {
  const ac = new AbortController();
  const timer = setTimeout(() => ac.abort(), ms);
  const res = await fetch(`${base}/events${query}`, { headers, signal: ac.signal });
  const frames = [];
  try {
    const reader = res.body.getReader();
    const dec = new TextDecoder();
    let buf = '';
    while (frames.length < want) {
      const { done, value } = await reader.read();
      if (done) break;
      buf += dec.decode(value, { stream: true });
      let idx;
      while ((idx = buf.indexOf('\n\n')) >= 0) {
        const raw = buf.slice(0, idx);
        buf = buf.slice(idx + 2);
        const fields = {};
        for (const line of raw.split('\n')) {
          const i = line.indexOf(':');
          if (i < 0) continue;
          const k = line.slice(0, i).trim();
          const v = line.slice(i + 1).replace(/^ /, '');
          fields[k] = (fields[k] ? `${fields[k]}\n` : '') + v;
        }
        if (fields.data !== undefined) frames.push({ id: fields.id, data: fields.data });
      }
    }
  } catch (e) {
    if (e.name !== 'AbortError') throw e;
  } finally {
    clearTimeout(timer);
    ac.abort();
  }
  return { status: res.status, type: res.headers.get('content-type'), frames };
}

test('the fixture list is discoverable and the fixtures load', async () => {
  await withServer(async (base) => {
    const res = await fetch(`${base}/fixtures`);
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.ok(body.fixtures.includes(FIXTURE));
    assert.ok(body.fixtures.includes('fallback_failure'));
    assert.ok(loadFixture(FIXTURE).length > 20, 'the demo fixture must be substantial');
  });
});

test('GET /events replays the fixture as one record per data: line, id = seq', async () => {
  await withServer(async (base) => {
    const fixture = loadFixture(FIXTURE);
    const { status, type, frames } = await readFrames(base, { query: `?fixture=${FIXTURE}&speed=0`, want: fixture.length });
    assert.equal(status, 200);
    assert.match(type, /text\/event-stream/);
    assert.equal(frames.length, fixture.length, 'every fixture record must be delivered');
    fixture.forEach((ev, i) => {
      const got = JSON.parse(frames[i].data);
      assert.equal(got.seq, ev.seq, `frame ${i} seq mismatch`);
      assert.equal(frames[i].id, String(ev.seq), 'the SSE id must be the envelope seq');
      assert.deepEqual(got.payload, ev.payload, `frame ${i} payload must be byte-identical`);
    });
  });
});

test('Last-Event-ID resumes after the last seen seq', async () => {
  await withServer(async (base) => {
    const total = loadFixture(FIXTURE).length;
    const after = 40;
    const { frames } = await readFrames(base, { query: `?fixture=${FIXTURE}&speed=0`, headers: { 'last-event-id': String(after) }, want: total });
    assert.equal(frames.length, total - after - 1, 'resume must skip exactly the records already seen');
    const seqs = frames.map((f) => JSON.parse(f.data).seq);
    assert.ok(seqs.every((s) => s > after), 'no record at or before Last-Event-ID may be resent');
    assert.equal(seqs[0], after + 1, 'resume must be gapless');
  });
});

test('POST /request feeds the live stream the page is watching', async () => {
  await withServer(async (base) => {
    // A viewer that has already seen everything: it must still get live events.
    const pending = readFrames(base, { query: '?fixture=water_reminder&speed=0&after=999999', want: 5, ms: 3000 });
    const res = await fetch(`${base}/request`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ text: 'remind me every 2 minutes to drink water' }) });
    assert.equal(res.status, 202);
    const ack = await res.json();
    assert.equal(ack.ok, true);
    const { frames } = await pending;
    const types = frames.map((f) => JSON.parse(f.data).payload.type);
    assert.deepEqual(types, ['RUN_STARTED', 'TEXT_MESSAGE_START', 'TEXT_MESSAGE_CONTENT', 'TEXT_MESSAGE_END', 'RUN_FINISHED']);
    assert.ok(JSON.parse(frames[2].data).payload.delta.includes('drink water'), 'the request text must reach the journal');
  });
});

test('the page and its modules are served locally, no CDN', async () => {
  await withServer(async (base) => {
    const page = await fetch(`${base}/`);
    assert.equal(page.status, 200);
    const html = await page.text();
    assert.match(html, /id="timeline"/);
    assert.match(html, /<script type="module" src="\/app\.js">/);
    assert.ok(!/https?:\/\/(?!127\.0\.0\.1)/.test(html), 'the page must not reference any external origin');
    for (const p of ['/app.js', '/render.mjs', '/dom.mjs', '/style.css']) {
      const r = await fetch(`${base}${p}`);
      assert.equal(r.status, 200, `${p} must be served`);
    }
  });
});

// ——— negatives ———

test('negative: an unknown fixture is refused by name', async () => {
  await withServer(async (base) => {
    const r = await fetch(`${base}/events?fixture=no_such_fixture&speed=0`);
    assert.equal(r.status, 404);
    assert.match((await r.json()).error, /no fixture named "no_such_fixture"/);
  });
});

test('negative: a path-traversal fixture name is refused, not resolved', async () => {
  await withServer(async (base) => {
    const r = await fetch(`${base}/events?fixture=../server&speed=0`);
    assert.equal(r.status, 400);
    assert.match((await r.json()).error, /must match \[a-z0-9_\]\+/);
  });
});

test('negative: an empty request is refused with the field named', async () => {
  await withServer(async (base) => {
    const r = await fetch(`${base}/request`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({}) });
    assert.equal(r.status, 400);
    assert.match((await r.json()).error, /"text" is required/);
    const r2 = await fetch(`${base}/request`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{ not json' });
    assert.equal(r2.status, 400);
    assert.match((await r2.json()).error, /not JSON/);
    const r3 = await fetch(`${base}/request`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ text: 'x'.repeat(600) }) });
    assert.equal(r3.status, 400);
    assert.match((await r3.json()).error, /longer than 500 characters/);
  });
});

test('negative: an unknown route answers 404 instead of serving the tree', async () => {
  await withServer(async (base) => {
    for (const p of ['/server.mjs', '/fixtures/water_reminder.jsonl', '/web/app.js']) {
      const r = await fetch(`${base}${p}`);
      assert.equal(r.status, 404, `${p} must not be served`);
      assert.match((await r.json()).error, /no route GET/);
    }
  });
});
