// Kimono — on-the-fly modules (protocol v1): validate and render module
// manifests exactly per docs/workbench/user-simulations/key-tests/modules/
// MODULE-PROTOCOL.md.
//
// A manifest is DATA, never code. The renderer treats every manifest string
// as untrusted text: strings only ever reach the page as text children of
// the injected h factory (createTextNode in the browser, plain-object
// text nodes in the headless test) — never as markup, never as a command.
//
// The block vocabulary is CLOSED (title, big_number, rows, action_button)
// and the action tools are the real agentd tool list. Anything outside
// either set refuses the WHOLE manifest: the card renders an explicit
// "unsupported module" state instead of silently half-rendering.
//
// An action button never calls a tool. It carries a plain-language user
// request (data-request) that app.js sends through the page's existing
// POST /request path, so the broker/confirm rules stay exactly as they are.

export const BLOCK_TYPES = ["title", "big_number", "rows", "action_button"];
export const TOOL_NAMES = [
  "phone_clock", "phone_status", "power_status", "power_plan",
  "service_create", "service_describe", "service_list", "service_stop",
  "app_launch",
];
export const INPUT_TYPES = ["string", "number", "minutes", "time", "boolean"];
export const INPUT_SOURCES = ["agent", "context", "tool"];

const isObj = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
const isNonEmptyStr = (v) => typeof v === "string" && v.trim() !== "";

// Structural validation against protocol v1. Returns { ok, errors } —
// every problem found, in plain words, so the refusal state can say why.
export function validateModule(m) {
  const errors = [];
  if (!isObj(m)) return { ok: false, errors: ["manifest is not a JSON object"] };
  if (!isNonEmptyStr(m.id)) errors.push("id is missing or empty");
  if (m.protocol !== "1") errors.push("protocol must be \"1\"");
  if (!isNonEmptyStr(m.user_purpose)) errors.push("user_purpose is missing or empty (required by the minimalism rule)");
  if (!isObj(m.trigger) || !isNonEmptyStr(m.trigger.when)) errors.push("trigger.when is missing or empty");

  const inputNames = new Set();
  if (!Array.isArray(m.inputs)) {
    errors.push("inputs must be an array");
  } else {
    for (const inp of m.inputs) {
      if (!isObj(inp) || !isNonEmptyStr(inp.name)) { errors.push("an input has no name"); continue; }
      inputNames.add(inp.name);
      if (!INPUT_TYPES.includes(inp.type)) errors.push("input " + inp.name + ": type " + JSON.stringify(inp.type) + " is not in the protocol set");
      if (!INPUT_SOURCES.includes(inp.source)) errors.push("input " + inp.name + ": source " + JSON.stringify(inp.source) + " is not in the protocol set");
    }
  }

  const actionIds = new Set();
  if (!Array.isArray(m.actions)) {
    errors.push("actions must be an array (it may be empty)");
  } else {
    for (const a of m.actions) {
      if (!isObj(a) || !isNonEmptyStr(a.id)) { errors.push("an action has no id"); continue; }
      actionIds.add(a.id);
      if (!TOOL_NAMES.includes(a.tool)) errors.push("action " + a.id + ": tool " + JSON.stringify(a.tool) + " is not a real agentd tool");
      if (a.tool === "app_launch" && isObj(a.args) && a.args.app !== undefined && a.args.app !== "maps") {
        errors.push("action " + a.id + ": app_launch args.app may only be \"maps\"");
      }
    }
  }

  if (!Array.isArray(m.layout) || m.layout.length === 0) {
    errors.push("layout must be a non-empty array of blocks");
  } else {
    m.layout.forEach((b, i) => {
      if (!isObj(b) || !BLOCK_TYPES.includes(b.block)) {
        errors.push("layout[" + i + "]: block " + JSON.stringify(isObj(b) ? b.block : b) + " is not in the closed vocabulary (" + BLOCK_TYPES.join(", ") + ")");
        return;
      }
      if (b.block === "title" && !isNonEmptyStr(b.text)) errors.push("layout[" + i + "]: title needs a text");
      if (b.block === "big_number" && !isNonEmptyStr(b.value)) errors.push("layout[" + i + "]: big_number needs a value");
      if (b.block === "rows" && !Array.isArray(b.items)) errors.push("layout[" + i + "]: rows needs an items array");
      if (b.block === "action_button") {
        if (!isNonEmptyStr(b.label)) errors.push("layout[" + i + "]: action_button needs a label");
        if (!actionIds.has(b.action)) errors.push("layout[" + i + "]: action_button references undeclared action " + JSON.stringify(b.action));
      }
    });
  }

  if (!isObj(m.lifetime) || !(typeof m.lifetime.max_ttl_minutes === "number" && m.lifetime.max_ttl_minutes > 0)) {
    errors.push("lifetime.max_ttl_minutes must be a positive number (every module expires)");
  }
  return { ok: errors.length === 0, errors };
}

// v1 data path: the values a card displays are the manifest's
// sample_inputs. Production manifests arrive with agent-filled inputs.
export function moduleValues(m) {
  return isObj(m) && isObj(m.sample_inputs) ? { ...m.sample_inputs } : {};
}

// A layout reference is an input name when the manifest declares that
// input, otherwise it is a literal. Values render as plain text.
function resolveRef(ref, m, values) {
  const declared = Array.isArray(m.inputs) && m.inputs.some((i) => i && i.name === ref);
  if (declared) {
    const v = values[ref];
    return v === undefined || v === null ? "" : String(v);
  }
  return ref === undefined || ref === null ? "" : String(ref);
}

// The plain-language user request a button sends. v1 wires app_launch
// actions only: "open maps and give me directions to <destination>",
// where the destination resolves from the action's args (an input name
// or a literal). Any other tool returns null — the button renders
// disabled rather than inventing a phrasing or calling the tool itself.
export function actionRequestText(m, action, values) {
  if (!isObj(action) || action.tool !== "app_launch") return null;
  const ref = isObj(action.args) ? action.args.destination : undefined;
  if (!isNonEmptyStr(ref)) return null;
  const dest = resolveRef(ref, m, values);
  if (!dest) return null;
  return "open maps and give me directions to " + dest;
}

function unsupportedCard(h, m, errors) {
  const id = isObj(m) && isNonEmptyStr(m.id) ? m.id : "unknown";
  return h("section", { class: "module-card module-unsupported", "data-module": id }, [
    h("span", { class: "module-unsupported-tag" }, "UNSUPPORTED MODULE"),
    h("span", { class: "module-unsupported-reason" }, errors.join("; ")),
  ]);
}

// Render one manifest to a card through the injected h factory.
// Invalid manifests and manifests missing a required input value render
// the explicit unsupported state — never a partial card.
export function renderModuleCard(h, m, values = moduleValues(m)) {
  const v = validateModule(m);
  if (!v.ok) return unsupportedCard(h, m, v.errors);
  const missing = (m.inputs || []).filter((i) => i.required && (values[i.name] === undefined || values[i.name] === null || values[i.name] === ""));
  if (missing.length) {
    return unsupportedCard(h, m, missing.map((i) => "no value for required input " + i.name));
  }
  const children = [];
  for (const b of m.layout) {
    if (b.block === "title") {
      children.push(h("div", { class: "module-title" }, resolveRef(b.text, m, values)));
    } else if (b.block === "big_number") {
      const parts = [h("span", { class: "module-big-value" }, resolveRef(b.value, m, values))];
      if (isNonEmptyStr(b.unit)) parts.push(h("span", { class: "module-big-unit" }, b.unit));
      if (isNonEmptyStr(b.caption)) parts.push(h("span", { class: "module-big-caption" }, resolveRef(b.caption, m, values)));
      children.push(h("div", { class: "module-big" }, parts));
    } else if (b.block === "rows") {
      children.push(h("div", { class: "module-rows" }, b.items.map((it) =>
        h("div", { class: "module-row" }, [
          h("span", { class: "module-row-label" }, String(it.label ?? "")),
          h("span", { class: "module-row-value" }, resolveRef(it.value, m, values)),
        ]))));
    } else if (b.block === "action_button") {
      const action = m.actions.find((a) => a.id === b.action);
      const req = actionRequestText(m, action, values);
      if (req) {
        children.push(h("button", { class: "module-btn", type: "button", "data-module-action": action.id, "data-request": req }, b.label));
      } else {
        children.push(h("button", { class: "module-btn", type: "button", disabled: "disabled", "data-module-action": b.action }, b.label + " (unavailable)"));
      }
    }
  }
  children.push(h("div", { class: "module-purpose" }, m.user_purpose));
  return h("section", { class: "module-card", "data-module": m.id }, children);
}

export function renderModules(h, manifests) {
  if (!Array.isArray(manifests)) return [];
  return manifests.map((m) => renderModuleCard(h, m));
}

// The honest empty state: no manifest has been emitted yet. This is
// the only thing the modules area shows before the first
// module_manifest journal record — sample data is never substituted.
export function renderModuleEmptyState(h) {
  return h("div", { class: "module-empty" }, "No modules yet — cards the agent composes will appear here when it emits them.");
}

// Render agent-emitted entries: [{ manifest, inputs }] in emission
// order, latest per id already deduped by the view. Agent-filled
// inputs ride beside the manifest; without them the card falls back
// to the manifest's own sample_inputs (test fixtures only).
export function renderEmittedModules(h, entries) {
  if (!Array.isArray(entries) || entries.length === 0) return [renderModuleEmptyState(h)];
  return entries.map((e) => {
    const m = isObj(e) && isObj(e.manifest) ? e.manifest : e;
    const inputs = isObj(e) && isObj(e.inputs) ? e.inputs : undefined;
    return inputs ? renderModuleCard(h, m, inputs) : renderModuleCard(h, m);
  });
}

// Parse the page's JSON data island: an array of manifests, or an
// object with a modules array. Anything else yields no modules.
export function parseModuleData(text) {
  if (!text) return [];
  try {
    const parsed = JSON.parse(text);
    if (Array.isArray(parsed)) return parsed;
    if (isObj(parsed) && Array.isArray(parsed.modules)) return parsed.modules;
    return [];
  } catch {
    return [];
  }
}
