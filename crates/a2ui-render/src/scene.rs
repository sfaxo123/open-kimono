//! Layout: an A2UI component tree → positioned items in one column.
//!
//! The phone is 480×800 in portrait with a finger on it, so the rules are few
//! and strict (VISION §8): one column, 48 px minimum touch targets, 8 px
//! rhythm, and everything the document asked for visible.
//!
//! Three things happen here that a naive renderer gets wrong:
//!
//! - **Progressive rendering.** A child id or a data path that has not
//!   arrived yet is a *placeholder*, not a crash and not a blank: the spec
//!   says components may reference things that show up later.
//! - **Unknown components are loud.** A type this client does not speak
//!   renders an `unsupported: X` chip and lands in [`Scene::unsupported`]. A
//!   silent hole in the middle of a screen is how an agent lies to you.
//! - **Cycles terminate.** A component that references itself gets a
//!   `cycle: id` chip, not a stack overflow.
//!
//! Text is wrapped here, at layout time, so the height a component reserves
//! is exactly the height [`crate::view`] draws — nothing measured twice.

use serde_json::Value;

use crate::surface::Surface;
use crate::value::{self, Ctx};

/// unifont cell width in orbclient.
pub const CHAR_W: i32 = 8;
/// unifont cell height in orbclient.
pub const LINE_H: i32 = 16;
/// Screen margin, and the basic rhythm unit.
pub const MARGIN: i32 = 8;
/// Vertical gap between stacked children.
pub const GAP: i32 = 8;
/// Minimum height of anything a finger is meant to hit.
pub const TOUCH_H: i32 = 48;
/// Inner padding of a card.
pub const CARD_PAD: i32 = 8;

/// A positioned box on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Where a component is being laid out: left edge, width, top edge. Carried as
/// one value so a recursive layout cannot transpose the numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub x: i32,
    pub w: i32,
    pub y: i32,
}

impl Slot {
    /// The same box moved down by `d` pixels.
    fn below(&self, d: i32) -> Slot {
        Slot { x: self.x, w: self.w, y: self.y + d }
    }
    /// The content box inside a container padded by `pad`.
    fn inset(&self, pad: i32) -> Slot {
        Slot { x: self.x + pad, w: (self.w - 2 * pad).max(1), y: self.y + pad }
    }
    fn rect(&self, h: i32) -> Rect {
        Rect { x: self.x, y: self.y, w: self.w, h }
    }
    /// How many characters fit in this width, with a little air.
    fn cols(&self) -> usize {
        ((self.w - 2 * MARGIN) / CHAR_W).max(1) as usize
    }
}

/// Text roles. orbclient has no glyph scaling, so a heading is styled the only
/// way this screen can: uppercase, orange, and (for h1) a rule under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Body,
    Caption,
    H1,
    H2,
    H3,
    H4,
    H5,
}

impl Style {
    fn heading(self) -> bool {
        !matches!(self, Style::Body | Style::Caption)
    }
    /// Extra vertical space above a heading, in px.
    pub fn top_space(self) -> i32 {
        match self {
            Style::H1 => 16,
            Style::H2 => 12,
            Style::H3 | Style::H4 | Style::H5 => 8,
            Style::Body | Style::Caption => 0,
        }
    }
    /// A rule drawn under the heading.
    pub fn rule(self) -> bool {
        matches!(self, Style::H1)
    }
}

/// How a button is dressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    Default,
    Primary,
    Borderless,
}

/// What a tap sends back to the agent (`common_types.json#/$defs/Action`).
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// `{"event": {"name": …, "context": {…}}}` — the agent handles it.
    Event { name: String, context: Value },
    /// `{"functionCall": {…}}` — the client tries it (see [`crate::events`]).
    Function { call: Value },
    /// The document gave the button no action: it is inert.
    None,
}

/// Why a box is a chip rather than real content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// The document asked for a component this client does not speak.
    Unsupported,
    /// A child id or data path the document has not delivered yet.
    Waiting,
    /// A component that references itself.
    Cycle,
    /// The document is wrong here (no root, a list bound to a non-list, …).
    Broken,
}

/// One drawn thing. The view never interprets the document — only these.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Text {
        lines: Vec<String>,
        style: Style,
    },
    Button {
        id: String,
        label: String,
        variant: ButtonVariant,
        action: Action,
    },
    Field {
        id: String,
        label: String,
        /// What is shown (already obscured if the field is a password).
        value: String,
        /// The model path the field writes back to, if it is bound.
        path: Option<String>,
        focused: bool,
        obscured: bool,
    },
    Progress {
        /// Normalised 0.0..=1.0; `None` = indeterminate or no data.
        fraction: Option<f64>,
        readout: String,
    },
    Image {
        description: String,
        url: String,
    },
    Divider,
    Chip {
        label: String,
        tone: Tone,
    },
    /// A card outline: drawn, never interactive.
    Frame,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub rect: Rect,
    pub kind: ItemKind,
}

/// A laid-out screen, ready to draw or to hit-test.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub width: i32,
    pub height: i32,
    /// Draw order: later items are on top.
    pub items: Vec<Item>,
    /// Component types this client could not render, deduplicated.
    pub unsupported: Vec<String>,
    /// Everything the renderer noticed about the document, in order.
    pub diagnostics: Vec<String>,
    /// Content taller than the canvas (the view marks the cut).
    pub overflow: bool,
    pub content_height: i32,
}

impl Scene {
    /// The topmost interactive item under a point, if any.
    pub fn hit_test(&self, x: i32, y: i32) -> Option<&Item> {
        self.items.iter().rev().find(|it| {
            matches!(&it.kind, ItemKind::Button { .. } | ItemKind::Field { .. }) && it.rect.contains(x, y)
        })
    }

    /// The focused text field, if any.
    pub fn focused_field(&self) -> Option<&Item> {
        self.items
            .iter()
            .rev()
            .find(|it| matches!(&it.kind, ItemKind::Field { focused: true, .. }))
    }
}

/// Layout options.
#[derive(Clone, Debug, Default)]
pub struct Options<'a> {
    /// Component id of the text input the keyboard is typing into.
    pub focus: Option<&'a str>,
}

/// Lay a surface out into a `width`×`height` scene.
pub fn layout(surface: &Surface, width: i32, height: i32) -> Scene {
    layout_with(surface, width, height, &Options::default())
}

pub fn layout_with(surface: &Surface, width: i32, height: i32, opts: &Options) -> Scene {
    let mut b = Builder {
        s: surface,
        items: Vec::new(),
        diag: surface.diagnostics.clone(),
        unsupported: Vec::new(),
        stack: Vec::new(),
        focus: opts.focus,
        content_height: 0,
    };
    let root_ctx = Ctx { data: &surface.data, item: None };
    let notice = Slot { x: MARGIN, w: width - 2 * MARGIN, y: MARGIN };
    if !surface.is_open() {
        b.diag.push("no surface is open".into());
        b.chip(notice.rect(LINE_H + 2 * MARGIN), Tone::Broken, "no surface open");
        b.content_height = MARGIN * 2 + LINE_H;
    } else if surface.root().is_none() {
        b.diag.push("document has no component with id \"root\"".into());
        b.chip(notice.rect(LINE_H + 2 * MARGIN), Tone::Broken, "no root component");
        b.content_height = MARGIN * 2 + LINE_H;
    } else {
        b.content_height = MARGIN + b.node("root", notice, root_ctx);
    }
    Scene {
        width,
        height,
        items: b.items,
        unsupported: b.unsupported,
        diagnostics: b.diag,
        overflow: b.content_height > height,
        content_height: b.content_height,
    }
}

struct Builder<'a> {
    s: &'a Surface,
    items: Vec<Item>,
    diag: Vec<String>,
    unsupported: Vec<String>,
    stack: Vec<String>,
    focus: Option<&'a str>,
    content_height: i32,
}

/// A container's children: a list of ids, or a template over a data list.
enum Children {
    Static(Vec<String>),
    Template { component_id: String, path: String },
}

impl Builder<'_> {
    /// Lay out one component; returns the height it used.
    fn node(&mut self, id: &str, at: Slot, ctx: Ctx<'_>) -> i32 {
        if self.stack.iter().any(|s| s == id) {
            self.diag.push(format!("cycle at `{id}`: {}", self.stack.join(" > ")));
            let h = LINE_H + 2 * MARGIN;
            self.chip(at.rect(h), Tone::Cycle, &format!("cycle: {id}"));
            return h;
        }
        let Some(c) = self.s.component(id) else {
            // Progressive rendering: the id is referenced, not delivered yet.
            self.diag.push(format!("`{id}` is referenced but not defined yet"));
            let h = LINE_H + 2 * MARGIN;
            self.chip(at.rect(h), Tone::Waiting, &format!("waiting: {id}"));
            return h;
        };
        let kind = c.get("component").and_then(Value::as_str).unwrap_or("?");
        self.stack.push(id.to_string());
        let h = self.component(id, c, kind, at, ctx);
        self.stack.pop();
        h.max(0)
    }

    fn component(&mut self, id: &str, c: &Value, kind: &str, at: Slot, ctx: Ctx<'_>) -> i32 {
        match kind {
            "Text" => self.text(c, at, ctx),
            "Column" => self.stack_children(id, at, ctx),
            "Card" => self.card(id, at, ctx),
            "Row" => self.row(id, at, ctx),
            "Divider" => {
                self.items.push(Item { rect: at.below(4).rect(1), kind: ItemKind::Divider });
                8
            }
            "List" => self.list(id, at, ctx),
            "Button" => self.button(id, c, at, ctx),
            "TextField" => self.field(id, c, at, ctx),
            // A2UI's basic catalog has no progress bar. The community
            // MaterialProgressBar (value 0..100, mode) is what agents actually
            // emit, so this client speaks all three names.
            "Progress" | "ProgressBar" | "MaterialProgressBar" => self.progress(c, at, ctx),
            "Image" => self.image(c, at, ctx),
            other => {
                if !self.unsupported.iter().any(|u| u == other) {
                    self.unsupported.push(other.to_string());
                }
                self.diag.push(format!("unsupported component `{other}` (id `{id}`)"));
                let h = LINE_H + 2 * MARGIN;
                self.chip(at.rect(h), Tone::Unsupported, &format!("unsupported: {other}"));
                h
            }
        }
    }

    // ---- properties ------------------------------------------------------

    fn str(&mut self, c: &Value, key: &str, ctx: Ctx<'_>) -> String {
        match c.get(key) {
            None => String::new(),
            Some(v) => value::text(v, &ctx, &mut self.diag),
        }
    }

    fn num(&mut self, c: &Value, key: &str, ctx: Ctx<'_>) -> Option<f64> {
        c.get(key).and_then(|v| value::number(v, &ctx, &mut self.diag))
    }

    /// Children from `children` (ids or a template) or from `child`.
    fn children(&mut self, c: &Value) -> Option<Children> {
        if let Some(list) = c.get("children") {
            match list {
                Value::Array(ids) => {
                    let mut out = Vec::new();
                    for v in ids {
                        match v.as_str() {
                            Some(s) => out.push(s.to_string()),
                            None => self.diag.push(format!("child entry {v} is not an id")),
                        }
                    }
                    return Some(Children::Static(out));
                }
                Value::Object(tmpl) => {
                    let cid = tmpl.get("componentId").and_then(Value::as_str);
                    let path = tmpl.get("path").and_then(Value::as_str);
                    match (cid, path) {
                        (Some(cid), Some(path)) => {
                            return Some(Children::Template { component_id: cid.to_string(), path: path.to_string() })
                        }
                        _ => self.diag.push("`children` template needs `componentId` and `path`".into()),
                    }
                }
                other => self.diag.push(format!("`children` must be a list or a template, got {other}")),
            }
        }
        c.get("child").and_then(Value::as_str).map(|cid| Children::Static(vec![cid.to_string()]))
    }

    /// Lay a container's children out top-to-bottom; returns the height used.
    fn stack_children(&mut self, parent: &str, at: Slot, ctx: Ctx<'_>) -> i32 {
        let Some(c) = self.s.component(parent) else {
            return 0;
        };
        let Some(children) = self.children(c) else {
            self.diag.push(format!("`{parent}` has no children"));
            return 0;
        };
        match children {
            Children::Static(ids) => {
                let mut used = 0;
                for (i, cid) in ids.iter().enumerate() {
                    if i > 0 {
                        used += GAP;
                    }
                    used += self.node(cid, at.below(used), ctx);
                }
                used
            }
            Children::Template { component_id, path } => {
                // Relative paths resolve against the item this template sits
                // in; the list data may not have arrived yet.
                let bound = value::bind(&path, &ctx).cloned();
                let list = match bound {
                    None => {
                        self.diag.push(format!("`{parent}` lists `{path}`, which has no data yet"));
                        let h = LINE_H + 2 * MARGIN;
                        self.chip(at.rect(h), Tone::Waiting, &format!("waiting: {path}"));
                        return h;
                    }
                    Some(Value::Array(items)) => items,
                    Some(other) => {
                        self.diag.push(format!("`{parent}` lists `{path}`, which is not a list: {other}"));
                        let h = LINE_H + 2 * MARGIN;
                        self.chip(at.rect(h), Tone::Broken, &format!("not a list: {path}"));
                        return h;
                    }
                };
                if list.is_empty() {
                    let h = LINE_H + 2 * MARGIN;
                    self.chip(at.rect(h), Tone::Broken, &format!("empty list: {path}"));
                    return h;
                }
                let mut used = 0;
                for (i, item) in list.iter().enumerate() {
                    if i > 0 {
                        used += GAP;
                    }
                    // Each row evaluates its bindings against its own item.
                    used += self.node(&component_id, at.below(used), ctx.with_item(item));
                }
                used
            }
        }
    }

    // ---- components ------------------------------------------------------

    fn text(&mut self, c: &Value, at: Slot, ctx: Ctx<'_>) -> i32 {
        let style = match c.get("variant").and_then(Value::as_str).unwrap_or("body") {
            "h1" => Style::H1,
            "h2" => Style::H2,
            "h3" => Style::H3,
            "h4" => Style::H4,
            "h5" => Style::H5,
            "caption" => Style::Caption,
            _ => Style::Body,
        };
        let raw = self.str(c, "text", ctx);
        let body = if style.heading() { raw.to_uppercase() } else { raw };
        let lines = wrap(&body, at.cols());
        let n = lines.len().max(1) as i32;
        let h = style.top_space() + n * LINE_H + if style.rule() { 6 } else { 0 };
        self.items.push(Item {
            rect: at.below(style.top_space()).rect(n * LINE_H),
            kind: ItemKind::Text { lines, style },
        });
        h
    }

    fn card(&mut self, id: &str, at: Slot, ctx: Ctx<'_>) -> i32 {
        let inner = self.stack_children(id, at.inset(CARD_PAD), ctx);
        let h = inner + 2 * CARD_PAD;
        // The outline is appended after its children: it is 1 px and never
        // covers text, and it cannot be sized before the children are laid out.
        self.items.push(Item { rect: at.rect(h), kind: ItemKind::Frame });
        h
    }

    fn row(&mut self, id: &str, at: Slot, ctx: Ctx<'_>) -> i32 {
        let Some(c) = self.s.component(id) else {
            return 0;
        };
        let Some(children) = self.children(c) else {
            return 0;
        };
        let Children::Static(ids) = children else {
            self.diag.push("`Row` with a template child list is not supported on this screen".into());
            let h = LINE_H + 2 * MARGIN;
            self.chip(at.rect(h), Tone::Unsupported, "row template");
            return h;
        };
        if ids.is_empty() {
            return 0;
        }
        let n = ids.len() as i32;
        let cell = (at.w / n).max(1);
        let mut tallest = 0;
        for (i, cid) in ids.iter().enumerate() {
            let i = i as i32;
            // The last cell absorbs the rounding remainder.
            let w = if i + 1 == n { at.w - cell * (n - 1) } else { cell };
            let cell_slot = Slot { x: at.x + cell * i, w, y: at.y };
            tallest = tallest.max(self.node(cid, cell_slot, ctx));
        }
        tallest
    }

    fn list(&mut self, id: &str, at: Slot, ctx: Ctx<'_>) -> i32 {
        // A horizontal list on a 480 px portrait screen becomes a row of
        // touch targets: the document asked for a list, the finger gets one.
        let horizontal =
            self.s.component(id).and_then(|c| c.get("direction")).and_then(Value::as_str) == Some("horizontal");
        if horizontal {
            return self.row(id, at, ctx);
        }
        self.stack_children(id, at, ctx)
    }

    fn button(&mut self, id: &str, c: &Value, at: Slot, ctx: Ctx<'_>) -> i32 {
        // A button's label lives in its `child` component (usually a Text).
        let label = match c.get("child").and_then(Value::as_str) {
            Some(cid) => match self.s.component(cid) {
                Some(child) => self.str(child, "text", ctx),
                None => {
                    self.diag.push(format!("button `{id}` points at `{cid}`, which is not defined yet"));
                    String::new()
                }
            },
            None => self.str(c, "text", ctx),
        };
        let variant = match c.get("variant").and_then(Value::as_str).unwrap_or("default") {
            "primary" => ButtonVariant::Primary,
            "borderless" => ButtonVariant::Borderless,
            _ => ButtonVariant::Default,
        };
        let action = match c.get("action") {
            None => {
                self.diag.push(format!("button `{id}` has no action"));
                Action::None
            }
            Some(a) => self.action(a, ctx),
        };
        let label = one_line(&label, ((at.w - 4 * MARGIN) / CHAR_W).max(1) as usize);
        let h = TOUCH_H.max(LINE_H + 2 * MARGIN);
        self.items.push(Item {
            rect: at.rect(h),
            kind: ItemKind::Button { id: id.to_string(), label, variant, action },
        });
        h
    }

    fn action(&mut self, a: &Value, ctx: Ctx<'_>) -> Action {
        if let Some(ev) = a.get("event") {
            let name = ev.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            if name.is_empty() {
                self.diag.push("action event has no `name`".into());
            }
            let context = ev.get("context").cloned().unwrap_or_else(|| Value::Object(Default::default()));
            // Resolve the context's dynamic values now, so a tap carries data
            // rather than paths.
            let context = value::resolve_tree(&context, &ctx, &mut self.diag);
            return Action::Event { name, context };
        }
        if let Some(f) = a.get("functionCall") {
            return Action::Function { call: f.clone() };
        }
        self.diag.push("action is neither `event` nor `functionCall`".into());
        Action::None
    }

    fn field(&mut self, id: &str, c: &Value, at: Slot, ctx: Ctx<'_>) -> i32 {
        let label = self.str(c, "label", ctx);
        let path = c
            .get("value")
            .and_then(|v| v.get("path"))
            .and_then(Value::as_str)
            .map(|p| p.to_string());
        // Typed-in text wins over the model, which wins over a literal.
        let value = match self.s.local_values.get(id) {
            Some(v) => v.clone(),
            None => self.str(c, "value", ctx),
        };
        let obscured = c.get("variant").and_then(Value::as_str) == Some("obscured");
        let shown: String = if obscured { value.chars().map(|_| '•').collect() } else { value };
        let h = LINE_H + 4 + TOUCH_H;
        self.items.push(Item {
            rect: at.rect(h),
            kind: ItemKind::Field {
                id: id.to_string(),
                label,
                value: shown,
                path,
                focused: self.focus == Some(id),
                obscured,
            },
        });
        h
    }

    fn progress(&mut self, c: &Value, at: Slot, ctx: Ctx<'_>) -> i32 {
        let min = self.num(c, "min", ctx).unwrap_or(0.0);
        let max = self.num(c, "max", ctx).unwrap_or(100.0);
        let label = self.str(c, "label", ctx);
        let indeterminate = c.get("mode").and_then(Value::as_str) == Some("indeterminate");
        let value = self.num(c, "value", ctx);

        let mut fraction = None;
        let mut readout = String::new();
        if indeterminate {
            readout = "…".into();
        } else {
            match value {
                Some(v) => {
                    let span = max - min;
                    if span <= 0.0 {
                        self.diag.push(format!("progress max {max} is not above min {min}"));
                    } else {
                        let f = ((v - min) / span).clamp(0.0, 1.0);
                        fraction = Some(f);
                        readout = format!("{}%", (f * 100.0).round() as i32);
                    }
                }
                None => self.diag.push("progress has no value yet".into()),
            }
        }

        let label_h = if label.is_empty() { 0 } else { LINE_H + 4 };
        if label_h > 0 {
            self.items.push(Item {
                rect: at.rect(LINE_H),
                kind: ItemKind::Text { lines: vec![one_line(&label, at.cols())], style: Style::Caption },
            });
        }
        self.items.push(Item {
            rect: at.below(label_h).rect(12),
            kind: ItemKind::Progress { fraction, readout },
        });
        label_h + 12 + 4
    }

    fn image(&mut self, c: &Value, at: Slot, ctx: Ctx<'_>) -> i32 {
        let url = c.get("url").and_then(Value::as_str).unwrap_or("").to_string();
        let description = self.str(c, "description", ctx);
        let box_h = match c.get("variant").and_then(Value::as_str).unwrap_or("mediumFeature") {
            "icon" | "avatar" => 40,
            "smallFeature" => 96,
            "header" => 120,
            "largeFeature" => 200,
            _ => 140,
        };
        let cap = if description.is_empty() { 0 } else { LINE_H + 4 };
        self.items.push(Item {
            rect: at.rect(box_h),
            kind: ItemKind::Image { description: description.clone(), url },
        });
        if cap > 0 {
            self.items.push(Item {
                rect: at.below(box_h + 2).rect(LINE_H),
                kind: ItemKind::Text { lines: vec![one_line(&description, at.cols())], style: Style::Caption },
            });
        }
        box_h + cap
    }

    fn chip(&mut self, rect: Rect, tone: Tone, label: &str) {
        self.items.push(Item { rect, kind: ItemKind::Chip { label: label.to_string(), tone } });
    }
}

/// Word-wrap to `cols` characters, hard-breaking anything longer. Char-based,
/// so a UTF-8 document cannot split a glyph mid-byte.
pub fn wrap(text: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        let mut len = 0usize;
        let mut emitted = false;
        for word in para.split_whitespace() {
            let wl = word.chars().count();
            if wl > cols {
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                    len = 0;
                    emitted = true;
                }
                let mut chunk = String::new();
                for ch in word.chars() {
                    if chunk.chars().count() == cols {
                        out.push(std::mem::take(&mut chunk));
                        emitted = true;
                    }
                    chunk.push(ch);
                }
                if !chunk.is_empty() {
                    out.push(chunk);
                    emitted = true;
                }
                continue;
            }
            if len + wl + usize::from(len > 0) > cols {
                out.push(std::mem::take(&mut line));
                len = 0;
                emitted = true;
            }
            if len > 0 {
                line.push(' ');
                len += 1;
            }
            line.push_str(word);
            len += wl;
        }
        // A paragraph with no words still gets a line; one already emitted by
        // a hard break must not be re-added whole.
        if !line.is_empty() {
            out.push(line);
        } else if !emitted {
            out.push(para.trim().to_string());
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Fit text onto one line, with an ellipsis when it does not fit.
pub fn one_line(text: &str, cols: usize) -> String {
    let cols = cols.max(1);
    let flat: String = text.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if flat.chars().count() <= cols {
        return flat;
    }
    let mut s: String = flat.chars().take(cols.saturating_sub(3)).collect();
    s.push_str("...");
    s
}
