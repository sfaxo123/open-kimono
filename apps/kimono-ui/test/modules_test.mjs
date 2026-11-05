// On-the-fly modules (protocol v1): the kimono page renders declarative
// manifests with a CLOSED block vocabulary, refuses anything outside it,
// escapes manifest strings as text, and wires action buttons to the
// page normal user-request path - never to a tool call.
//
//   node --test test/modules_test.mjs

import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { validateModule, renderModuleCard, renderModules, renderEmittedModules, renderModuleEmptyState, actionRequestText, moduleValues, parseModuleData } from "../web/modules.mjs";
import { newView, apply, moduleEntries } from "../web/render.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const FIX = path.join(HERE, "..", "fixtures", "modules");

function h(tag, attrs = {}, children = []) {
  const node = { tag, attrs: attrs || {}, children: [] };
  for (const c of [].concat(children === null ? [] : children)) {
    if (c === null || c === undefined) continue;
    node.children.push(typeof c === "string" ? { tag: "#text", text: c } : c);
  }
  return node;
}
function textOf(node) {
  if (node.tag === "#text") return node.text;
  return node.children.map(textOf).join(" ");
}
function findAll(node, pred, out = []) {
  if (pred(node)) out.push(node);
  for (const c of node.children || []) findAll(c, pred, out);
  return out;
}
const hasClass = (node, cls) => String((node.attrs || {}).class || "").split(" ").includes(cls);
const load = (name) => JSON.parse(readFileSync(path.join(FIX, name + ".json"), "utf8"));

const GOOD = ["eta-card", "flight-card", "maps-card"];

test("the three adopted sample manifests validate against protocol v1", () => {
  for (const name of GOOD) {
    const v = validateModule(load(name));
    assert.ok(v.ok, name + " should validate, got: " + v.errors.join("; "));
  }
});

test("all three sample manifests render their sample values", () => {
  const eta = textOf(renderModuleCard(h, load("eta-card")));
  assert.match(eta, /San Francisco International Airport \(SFO\)/);
  assert.match(eta, /55/);
  assert.match(eta, /I-880/);
  assert.match(eta, /No tolls on the fast route/);
  const flight = textOf(renderModuleCard(h, load("flight-card")));
  assert.match(flight, /UA482 from SFO/);
  assert.match(flight, /16:50/);
  assert.match(flight, /17:45/);
  assert.match(flight, /18:30/);
  const maps = textOf(renderModuleCard(h, load("maps-card")));
  assert.match(maps, /San Francisco/);
  for (const name of GOOD) {
    const node = renderModuleCard(h, load(name));
    assert.equal(node.tag, "section", name + " renders a section card");
    assert.ok(hasClass(node, "module-card"), name + " carries the module-card class");
    assert.equal(node.attrs["data-module"], name);
  }
});

test("an unknown block is refused whole, never half-rendered", () => {
  const m = load("bad_unknown_block");
  const v = validateModule(m);
  assert.ok(!v.ok, "unknown block must fail validation");
  assert.ok(v.errors.some((e) => e.includes("video")), "the refusal names the offending block: " + v.errors.join("; "));
  const node = renderModuleCard(h, m);
  assert.ok(hasClass(node, "module-unsupported"), "renders the explicit unsupported state");
  assert.match(textOf(node), /UNSUPPORTED MODULE/);
  assert.equal(findAll(node, (n) => hasClass(n, "module-title")).length, 0, "no part of a refused manifest renders as a card");
});

test("a manifest string with markup stays text (escaped, never markup)", () => {
  const m = load("bad_markup_title");
  assert.ok(validateModule(m).ok, "markup in a string is data, so the manifest itself is valid");
  const node = renderModuleCard(h, m);
  assert.equal(findAll(node, (n) => n.tag === "img" || n.tag === "script").length, 0, "no element is ever built from a manifest string");
  const literal = "<img src=x onerror=alert(1)> San Francisco";
  assert.ok(textOf(node).includes(literal), "the string renders verbatim as text");
  const carriers = findAll(node, (n) => n.tag === "#text" && n.text === literal);
  assert.equal(carriers.length, 1, "the string arrives as exactly one text node");
});

test("a button carries the right request text and nothing else", () => {
  const mapsNode = renderModuleCard(h, load("maps-card"));
  const mapsBtns = findAll(mapsNode, (n) => n.tag === "button");
  assert.equal(mapsBtns.length, 1);
  assert.equal(mapsBtns[0].attrs["data-request"], "open maps and give me directions to San Francisco");
  assert.equal(mapsBtns[0].attrs["data-module-action"], "open_maps");
  const flightNode = renderModuleCard(h, load("flight-card"));
  const flightBtns = findAll(flightNode, (n) => n.tag === "button");
  assert.equal(flightBtns.length, 1);
  assert.equal(flightBtns[0].attrs["data-request"], "open maps and give me directions to San Francisco International Airport (SFO)");
  const etaBtns = findAll(renderModuleCard(h, load("eta-card")), (n) => n.tag === "button");
  assert.equal(etaBtns.length, 0, "eta-card declares no actions, so it renders no buttons");
  const m = load("maps-card");
  assert.equal(actionRequestText(m, m.actions[0], moduleValues(m)), "open maps and give me directions to San Francisco");
});

test("an invented tool and a missing purpose are both refused", () => {
  const tool = validateModule(load("bad_unknown_tool"));
  assert.ok(!tool.ok && tool.errors.some((e) => e.includes("shell_exec")), "unknown tool refused by name");
  const purpose = validateModule(load("bad_missing_purpose"));
  assert.ok(!purpose.ok && purpose.errors.some((e) => e.includes("user_purpose")), "missing user_purpose refused");
  for (const name of ["bad_unknown_tool", "bad_missing_purpose"]) {
    assert.ok(hasClass(renderModuleCard(h, load(name)), "module-unsupported"), name + " renders the unsupported state");
  }
});

test("the page ships no sample data island — modules come only from the journal", () => {
  const html = readFileSync(path.join(HERE, "..", "web", "index.html"), "utf8");
  assert.ok(html.includes("id=\"modules\""), "the page has a modules area");
  assert.ok(!html.includes("module-data"), "index.html must not carry a sample module-data island");
  assert.ok(!html.includes("eta-card") && !html.includes("flight-card"), "no sample manifest may ship in the page");
  // parseModuleData is retained as a pure parser (other tooling uses
  // the shape), but nothing in the page feeds it sample data.
  assert.deepEqual(parseModuleData(""), []);
  assert.deepEqual(parseModuleData("not json"), []);
});

test("with nothing emitted, the modules area is the honest empty state", () => {
  const view = newView();
  assert.deepEqual(moduleEntries(view), []);
  const nodes = renderEmittedModules(h, moduleEntries(view));
  assert.equal(nodes.length, 1);
  assert.ok(hasClass(nodes[0], "module-empty"));
  assert.match(textOf(nodes[0]), /No modules yet/);
  assert.equal(findAll(nodes[0], (n) => hasClass(n, "module-card")).length, 0, "empty state is not a fake card");
  assert.match(textOf(renderModuleEmptyState(h)), /No modules yet/);
});

// Captured emitted manifests: these fixtures are verbatim
// module_manifest values agentd journaled during a host proof run
// (fake brain + stub gio, session sess-capture-1). Validating and
// rendering them here with the REAL modules.mjs validator is the
// conformance proof — the same code the page runs.
const EMITTED = ["emitted-destination-card", "emitted-session-card"];

test("captured agent-emitted manifests validate with the real validator", () => {
  for (const name of EMITTED) {
    const entry = load(name);
    const v = validateModule(entry.manifest);
    assert.ok(v.ok, name + " must validate, got: " + v.errors.join("; "));
    assert.ok(entry.manifest.user_purpose.trim() !== "", name + " needs a user_purpose");
    for (const inp of entry.manifest.inputs.filter((i) => i.required)) {
      assert.ok(entry.inputs[inp.name] !== undefined && entry.inputs[inp.name] !== "", name + " missing value for " + inp.name);
    }
  }
});

test("captured emitted manifests render their agent-filled values", () => {
  const dest = load("emitted-destination-card");
  const node = renderModuleCard(h, dest.manifest, dest.inputs);
  assert.ok(hasClass(node, "module-card") && !hasClass(node, "module-unsupported"));
  assert.equal(node.attrs["data-module"], "destination-card");
  const text = textOf(node);
  assert.match(text, /San Francisco/);
  assert.match(text, /geo:0,0\?q=San%20Francisco/);
  assert.match(text, /run-1793783038129582-0/);
  const btns = findAll(node, (n) => n.tag === "button");
  assert.equal(btns.length, 1);
  assert.equal(btns[0].attrs["data-request"], "open maps and give me directions to San Francisco");
  const sess = load("emitted-session-card");
  const snode = renderModuleCard(h, sess.manifest, sess.inputs);
  assert.ok(hasClass(snode, "module-card"));
  const stext = textOf(snode);
  assert.match(stext, /sess-capture-1/);
  assert.match(stext, /2/);
  assert.match(stext, /closed/);
});

let modSeq = 0;
function moduleEvent(entry) {
  modSeq += 1;
  return { seq: modSeq, ts_us: modSeq * 1000000, service: "agentd", trace_id: null, span_id: null, parent_span_id: null, payload: { kind: "ag_ui", type: "CUSTOM", name: "module_manifest", value: entry } };
}

test("journaled module_manifest records drive the modules view; latest per id wins", () => {
  const view = newView();
  const dest = load("emitted-destination-card");
  const sess = load("emitted-session-card");
  const r = apply(view, moduleEvent(dest));
  assert.equal(r.action, "state");
  apply(view, moduleEvent(sess));
  assert.deepEqual(moduleEntries(view).map((e) => e.manifest.id), ["destination-card", "session-card"]);
  const nodes = renderEmittedModules(h, moduleEntries(view));
  assert.equal(nodes.length, 2);
  assert.ok(nodes.every((n) => hasClass(n, "module-card")));
  // Re-emission with a new turn count updates the same card, no duplicate.
  const updated = JSON.parse(JSON.stringify(sess));
  updated.inputs.turn_count = 3;
  apply(view, moduleEvent(updated));
  assert.equal(moduleEntries(view).length, 2, "re-emission must not duplicate the card");
  assert.match(textOf(renderEmittedModules(h, moduleEntries(view))[1]), /3/);
});

test("an invalid emitted manifest renders unsupported, and emitted strings stay text", () => {
  const view = newView();
  const bad = load("emitted-destination-card");
  const badCopy = JSON.parse(JSON.stringify(bad));
  delete badCopy.manifest.user_purpose;
  apply(view, moduleEvent(badCopy));
  const nodes = renderEmittedModules(h, moduleEntries(view));
  assert.ok(hasClass(nodes[0], "module-unsupported"));
  assert.match(textOf(nodes[0]), /UNSUPPORTED MODULE/);
  const markup = load("emitted-destination-card");
  const mCopy = JSON.parse(JSON.stringify(markup));
  mCopy.inputs.destination = "<img src=x onerror=alert(1)> San Francisco";
  const node = renderModuleCard(h, mCopy.manifest, mCopy.inputs);
  assert.equal(findAll(node, (n) => n.tag === "img" || n.tag === "script").length, 0);
  assert.ok(textOf(node).includes("<img src=x onerror=alert(1)> San Francisco"));
});
