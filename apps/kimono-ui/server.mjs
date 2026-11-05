#!/usr/bin/env node
// Kimono (Track A2) — the fixture replay server.
//
// A zero-dependency node server that lets the kimono page be developed and
// tested before agentd exists: it serves the static page and replays a
// recorded trace-events fixture as Server-Sent Events. The SSE contract is
// the one agentd must satisfy: one journal record per `data:` line, `id:` =
// the envelope `seq`, and `Last-Event-ID` resumes after that seq.
//
//   node server.mjs [--port 8790] [--fixture water_reminder] [--speed 120]
//
// Binds 127.0.0.1 only (PLAN phone-safety rule: the kimono view is loopback
// or the phone's Meshnet address, never 0.0.0.0).

import http from 'node:http';
import { readFile, readdir } from 'node:fs/promises';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const WEB = path.join(HERE, 'web');
const FIXTURES = path.join(HERE, 'fixtures');

const STATIC = {
  '/': ['index.html', 'text/html; charset=utf-8'],
  '/index.html': ['index.html', 'text/html; charset=utf-8'],
  '/app.js': ['app.js', 'text/javascript; charset=utf-8'],
  '/render.mjs': ['render.mjs', 'text/javascript; charset=utf-8'],
  '/dom.mjs': ['dom.mjs', 'text/javascript; charset=utf-8'],
  '/modules.mjs': ['modules.mjs', 'text/javascript; charset=utf-8'],
  '/style.css': ['style.css', 'text/css; charset=utf-8'],
};

function arg(name, dflt) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] !== undefined ? process.argv[i + 1] : dflt;
}

const PORT = Number(arg('port', process.env.KIMONO_PORT || 8790));
const DEFAULT_FIXTURE = arg('fixture', 'water_reminder');
const SPEED = Number(arg('speed', '120'));

export function listFixtures() {
  if (!existsSync(FIXTURES)) return [];
  return readdirSync(FIXTURES)
    .filter((f) => /^[a-z0-9_]+\.jsonl$/.test(f))
    .map((f) => f.replace(/\.jsonl$/, ''));
}

export function loadFixture(name) {
  if (!/^[a-z0-9_]+$/.test(name)) {
    const err = new Error(`fixture name must match [a-z0-9_]+, got ${JSON.stringify(name)}`);
    err.status = 400;
    throw err;
  }
  const file = path.join(FIXTURES, `${name}.jsonl`);
  if (!existsSync(file)) {
    const err = new Error(`no fixture named ${JSON.stringify(name)} (have: ${listFixtures().join(', ')})`);
    err.status = 404;
    throw err;
  }
  const lines = readFileSync(file, 'utf8').split('\n').filter((l) => l.trim() !== '');
  return lines.map((l) => JSON.parse(l));
}

// The live feed: fixture replay, then anything POSTed to /request.
const feed = { events: [], nextSeq: 0, clients: new Set() };

export function seed(fixtureName) {
  feed.events = loadFixture(fixtureName);
  feed.nextSeq = feed.events.length ? feed.events[feed.events.length - 1].seq + 1 : 0;
  return feed.events.length;
}

export function publish(payloads) {
  const ts = Date.now() * 1000;
  const added = payloads.map((payload, i) => {
    const ev = { seq: feed.nextSeq + i, ts_us: ts, service: 'kimono-replay', payload };
    return ev;
  });
  feed.nextSeq += added.length;
  feed.events.push(...added);
  for (const res of feed.clients) for (const ev of added) writeEvent(res, ev);
  return added;
}

function writeEvent(res, ev) {
  res.write(`id: ${ev.seq}\ndata: ${JSON.stringify(ev)}\n\n`);
}

function sseHead(res) {
  res.writeHead(200, {
    'content-type': 'text/event-stream; charset=utf-8',
    'cache-control': 'no-store',
    connection: 'keep-alive',
    'access-control-allow-origin': 'http://127.0.0.1',
  });
}

async function handle(req, res) {
  const url = new URL(req.url, 'http://127.0.0.1');
  const route = url.pathname;

  if (route === '/fixtures' && req.method === 'GET') {
    res.writeHead(200, { 'content-type': 'application/json' });
    return res.end(JSON.stringify({ fixtures: listFixtures(), default: DEFAULT_FIXTURE }));
  }

  if (route === '/events' && req.method === 'GET') {
    const name = url.searchParams.get('fixture') || DEFAULT_FIXTURE;
    let events;
    try {
      events = loadFixture(name);
    } catch (e) {
      res.writeHead(e.status || 500, { 'content-type': 'application/json' });
      return res.end(JSON.stringify({ error: e.message }));
    }
    const speed = Number(url.searchParams.get('speed') ?? SPEED);
    const after = Number(req.headers['last-event-id'] ?? url.searchParams.get('after') ?? -1);
    sseHead(res);
    feed.clients.add(res);
    const mine = events.filter((e) => e.seq > after);
    for (const ev of mine) {
      writeEvent(res, ev);
      if (speed > 0) await new Promise((r) => setTimeout(r, speed));
    }
    req.on('close', () => feed.clients.delete(res));
    return;
  }

  if (route === '/request' && req.method === 'POST') {
    let body = '';
    for await (const chunk of req) body += chunk;
    let text;
    let confirm;
    try {
      const parsed = JSON.parse(body || '{}');
      text = parsed.text;
      confirm = parsed.confirm;
    } catch {
      res.writeHead(400, { 'content-type': 'application/json' });
      return res.end(JSON.stringify({ error: 'body is not JSON' }));
    }
    if (typeof text !== 'string' || text.trim() === '') {
      res.writeHead(400, { 'content-type': 'application/json' });
      return res.end(JSON.stringify({ error: 'field "text" is required and must be non-empty' }));
    }
    if (text.length > 500) {
      res.writeHead(400, { 'content-type': 'application/json' });
      return res.end(JSON.stringify({ error: 'field "text" is longer than 500 characters' }));
    }
    // W6: a Confirm re-POST carries confirm = the blocked label, exactly
    // as agentd parses it (server.rs). It rides in the run input so the
    // replay journal shows what was authorised; anything else is refused.
    if (confirm !== undefined && confirm !== null && (typeof confirm !== 'string' || confirm.trim() === '')) {
      res.writeHead(400, { 'content-type': 'application/json' });
      return res.end(JSON.stringify({ error: 'field "confirm" must be a non-empty string naming the action being confirmed' }));
    }
    const id = `replay-${feed.nextSeq}`;
    const input = confirm ? { text, confirm } : { text };
    publish([
      { kind: 'ag_ui', type: 'RUN_STARTED', threadId: 'replay', runId: id, input },
      { kind: 'ag_ui', type: 'TEXT_MESSAGE_START', messageId: id, role: 'user' },
      { kind: 'ag_ui', type: 'TEXT_MESSAGE_CONTENT', messageId: id, delta: text },
      { kind: 'ag_ui', type: 'TEXT_MESSAGE_END', messageId: id },
      { kind: 'ag_ui', type: 'RUN_FINISHED', threadId: 'replay', runId: id, outcome: { type: 'success', pendingToolCallIds: [] } },
    ]);
    res.writeHead(202, { 'content-type': 'application/json' });
    return res.end(JSON.stringify(confirm ? { ok: true, runId: id, confirm } : { ok: true, runId: id }));
  }

  const stat = STATIC[route];
  if (stat && req.method === 'GET') {
    const [file, type] = stat;
    try {
      const bytes = await readFile(path.join(WEB, file));
      res.writeHead(200, { 'content-type': type, 'cache-control': 'no-store' });
      return res.end(bytes);
    } catch (e) {
      res.writeHead(500, { 'content-type': 'application/json' });
      return res.end(JSON.stringify({ error: `cannot read ${file}: ${e.message}` }));
    }
  }

  res.writeHead(404, { 'content-type': 'application/json' });
  return res.end(JSON.stringify({ error: `no route ${req.method} ${route}` }));
}

export function start({ port = PORT, fixture = DEFAULT_FIXTURE } = {}) {
  const n = seed(fixture);
  const server = http.createServer((req, res) => {
    handle(req, res).catch((e) => {
      if (!res.headersSent) res.writeHead(500, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ error: e.message }));
    });
  });
  return new Promise((resolve) => {
    server.listen(port, '127.0.0.1', () => resolve({ server, port: server.address().port, events: n }));
  });
}

// Only run the server when invoked directly, so the tests can import it.
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { server, port, events } = await start();
  console.log(`kimono replay on http://127.0.0.1:${port} — fixture ${DEFAULT_FIXTURE} (${events} events, ${SPEED} ms/event)`);
  for (const sig of ['SIGINT', 'SIGTERM']) process.on(sig, () => { server.close(); process.exit(0); });
}
