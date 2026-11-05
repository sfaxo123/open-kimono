//! The deterministic expression subset shared by `condition_cel`, `when_cel`
//! and `{{ ... }}` templates (SERVICE-IR.md §3.2: "no floating-point NaN
//! non-determinism, no syscalls, no wall-clock reads").
//!
//! Grammar (CEL-shaped, deliberately tiny):
//! literals (`1`, `2.5`, `'s'`, `"s"`, `true`, `false`, `null`), dotted paths
//! (`state.armed`, `trigger.payload.capacity_pct`), unary `!` `-`, binary
//! `* / %`, `+ -`, `< <= > >=`, `== !=`, `&&`, `||`, parentheses.
//! Undefined paths are errors (strict, like Goose's `UndefinedBehavior::Strict`).

use serde_json::{Number, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Lit(Value),
    Path(Vec<String>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExprError(pub String);

impl std::fmt::Display for ExprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, ExprError> {
    Err(ExprError(msg.into()))
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(Value),
    Str(String),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
    Dot,
}

const OPS: [&str; 15] = ["&&", "||", "==", "!=", "<=", ">=", "<", ">", "+", "-", "*", "/", "%", "!", "="];

fn lex(src: &str) -> Result<Vec<Tok>, ExprError> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            out.push(Tok::Num(parse_number(&text)?));
        } else if c == '\'' || c == '"' {
            let quote = c;
            i += 1;
            let mut s = String::new();
            loop {
                match chars.get(i) {
                    None => return err(format!("unterminated string literal in `{src}`")),
                    Some(&ch) if ch == quote => break,
                    Some('\\') if i + 1 < chars.len() => {
                        s.push(chars[i + 1]);
                        i += 1;
                    }
                    Some(&ch) => s.push(ch),
                }
                i += 1;
            }
            i += 1;
            out.push(Tok::Str(s));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else if c == '(' {
            out.push(Tok::LParen);
            i += 1;
        } else if c == ')' {
            out.push(Tok::RParen);
            i += 1;
        } else if c == '.' {
            out.push(Tok::Dot);
            i += 1;
        } else {
            let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
            let Some(op) = OPS.iter().find(|op| rest.starts_with(**op)) else {
                return err(format!("unexpected character `{c}` in `{src}`"));
            };
            if *op == "=" {
                return err(format!("`=` is not an operator (use `==`) in `{src}`"));
            }
            out.push(Tok::Op(op));
            i += op.len();
        }
    }
    Ok(out)
}

fn parse_number(text: &str) -> Result<Value, ExprError> {
    if let Ok(i) = text.parse::<i64>() {
        return Ok(Value::Number(i.into()));
    }
    match text.parse::<f64>().ok().and_then(Number::from_f64) {
        Some(n) => Ok(Value::Number(n)),
        None => err(format!("bad number literal `{text}`")),
    }
}

/// Parse an expression; the whole input must be consumed.
pub fn parse(src: &str) -> Result<Expr, ExprError> {
    let toks = lex(src)?;
    if toks.is_empty() {
        return err("empty expression");
    }
    let mut p = Parser { toks, pos: 0, src };
    let e = p.binary(0)?;
    if p.pos != p.toks.len() {
        return err(format!("unexpected trailing input in `{src}`"));
    }
    Ok(e)
}

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    src: &'a str,
}

fn binop(op: &str) -> Option<(BinOp, u8)> {
    Some(match op {
        "||" => (BinOp::Or, 1),
        "&&" => (BinOp::And, 2),
        "==" => (BinOp::Eq, 3),
        "!=" => (BinOp::Ne, 3),
        "<" => (BinOp::Lt, 4),
        "<=" => (BinOp::Le, 4),
        ">" => (BinOp::Gt, 4),
        ">=" => (BinOp::Ge, 4),
        "+" => (BinOp::Add, 5),
        "-" => (BinOp::Sub, 5),
        "*" => (BinOp::Mul, 6),
        "/" => (BinOp::Div, 6),
        "%" => (BinOp::Rem, 6),
        _ => return None,
    })
}

impl Parser<'_> {
    fn binary(&mut self, min_prec: u8) -> Result<Expr, ExprError> {
        let mut lhs = self.unary()?;
        while let Some(Tok::Op(op)) = self.toks.get(self.pos) {
            let Some((bop, prec)) = binop(op) else { break };
            if prec < min_prec {
                break;
            }
            self.pos += 1;
            let rhs = self.binary(prec + 1)?;
            lhs = Expr::Bin(bop, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, ExprError> {
        let tok = self.toks.get(self.pos).cloned();
        self.pos += 1;
        match tok {
            Some(Tok::Op("!")) => Ok(Expr::Not(Box::new(self.unary()?))),
            Some(Tok::Op("-")) => Ok(Expr::Neg(Box::new(self.unary()?))),
            Some(Tok::Num(v)) => Ok(Expr::Lit(v)),
            Some(Tok::Str(s)) => Ok(Expr::Lit(Value::String(s))),
            Some(Tok::LParen) => {
                let e = self.binary(0)?;
                if self.toks.get(self.pos) != Some(&Tok::RParen) {
                    return err(format!("missing `)` in `{}`", self.src));
                }
                self.pos += 1;
                Ok(e)
            }
            Some(Tok::Ident(id)) => match id.as_str() {
                "true" => Ok(Expr::Lit(Value::Bool(true))),
                "false" => Ok(Expr::Lit(Value::Bool(false))),
                "null" => Ok(Expr::Lit(Value::Null)),
                _ => {
                    let mut path = vec![id];
                    while self.toks.get(self.pos) == Some(&Tok::Dot) {
                        match self.toks.get(self.pos + 1) {
                            Some(Tok::Ident(seg)) => path.push(seg.clone()),
                            _ => return err(format!("expected a name after `.` in `{}`", self.src)),
                        }
                        self.pos += 2;
                    }
                    Ok(Expr::Path(path))
                }
            },
            Some(t) => err(format!("unexpected token {t:?} in `{}`", self.src)),
            None => err(format!("unexpected end of expression `{}`", self.src)),
        }
    }
}

/// Resolves the root of a dotted path (`state`, `params`, `trigger`, ...).
pub trait Scope {
    fn root(&self, name: &str) -> Option<Value>;
}

impl Expr {
    /// Every dotted path the expression reads (for static validation).
    pub fn paths(&self, out: &mut Vec<Vec<String>>) {
        match self {
            Expr::Lit(_) => {}
            Expr::Path(p) => out.push(p.clone()),
            Expr::Not(e) | Expr::Neg(e) => e.paths(out),
            Expr::Bin(_, a, b) => {
                a.paths(out);
                b.paths(out);
            }
        }
    }

    pub fn eval(&self, scope: &dyn Scope) -> Result<Value, ExprError> {
        match self {
            Expr::Lit(v) => Ok(v.clone()),
            Expr::Path(p) => lookup(scope, p),
            Expr::Not(e) => match e.eval(scope)? {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                v => err(format!("`!` needs a boolean, got {v}")),
            },
            Expr::Neg(e) => arith(BinOp::Sub, &Value::from(0), &e.eval(scope)?),
            Expr::Bin(BinOp::And, a, b) => Ok(Value::Bool(truthy(&a.eval(scope)?)? && truthy(&b.eval(scope)?)?)),
            Expr::Bin(BinOp::Or, a, b) => Ok(Value::Bool(truthy(&a.eval(scope)?)? || truthy(&b.eval(scope)?)?)),
            Expr::Bin(op, a, b) => {
                let (a, b) = (a.eval(scope)?, b.eval(scope)?);
                match op {
                    BinOp::Eq => Ok(Value::Bool(json_eq(&a, &b))),
                    BinOp::Ne => Ok(Value::Bool(!json_eq(&a, &b))),
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => compare(*op, &a, &b),
                    BinOp::Add if a.is_string() || b.is_string() => match (&a, &b) {
                        (Value::String(x), Value::String(y)) => Ok(Value::String(format!("{x}{y}"))),
                        _ => err(format!("cannot add {a} and {b}")),
                    },
                    _ => arith(*op, &a, &b),
                }
            }
        }
    }

    /// Evaluate as a guard; anything but a boolean is an error.
    pub fn eval_bool(&self, scope: &dyn Scope) -> Result<bool, ExprError> {
        truthy(&self.eval(scope)?)
    }
}

fn truthy(v: &Value) -> Result<bool, ExprError> {
    v.as_bool().ok_or_else(|| ExprError(format!("expected a boolean, got {v}")))
}

fn lookup(scope: &dyn Scope, path: &[String]) -> Result<Value, ExprError> {
    let mut cur = scope.root(&path[0]).ok_or_else(|| ExprError(format!("undefined name `{}`", path[0])))?;
    for (i, seg) in path.iter().enumerate().skip(1) {
        cur = match cur.get(seg.as_str()) {
            Some(v) => v.clone(),
            None => return err(format!("undefined path `{}`", path[..=i].join("."))),
        };
    }
    Ok(cur)
}

/// Numeric-aware equality: `19 == 19.0`; other types compare structurally.
pub fn json_eq(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) if a.is_number() && b.is_number() => x == y,
        _ => a == b,
    }
}

/// `expected` is contained in `actual`: objects by key (recursively), other
/// values by [`json_eq`]. Used for bus data filters and simulator assertions.
pub fn json_subset(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => e.iter().all(|(k, v)| a.get(k).is_some_and(|av| json_subset(v, av))),
        _ => json_eq(expected, actual),
    }
}

fn compare(op: BinOp, a: &Value, b: &Value) -> Result<Value, ExprError> {
    let ord = match (a, b) {
        (Value::Number(_), Value::Number(_)) => a.as_f64().unwrap().partial_cmp(&b.as_f64().unwrap()),
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    };
    let Some(ord) = ord else { return err(format!("cannot compare {a} and {b}")) };
    Ok(Value::Bool(match op {
        BinOp::Lt => ord.is_lt(),
        BinOp::Le => ord.is_le(),
        BinOp::Gt => ord.is_gt(),
        _ => ord.is_ge(),
    }))
}

fn arith(op: BinOp, a: &Value, b: &Value) -> Result<Value, ExprError> {
    let (Some(x), Some(y)) = (a.as_number(), b.as_number()) else {
        return err(format!("arithmetic needs numbers, got {a} and {b}"));
    };
    if let (Some(x), Some(y)) = (x.as_i64(), y.as_i64()) {
        let r = match op {
            BinOp::Add => x.checked_add(y),
            BinOp::Sub => x.checked_sub(y),
            BinOp::Mul => x.checked_mul(y),
            BinOp::Rem => x.checked_rem(y),
            BinOp::Div if y != 0 && x % y == 0 => Some(x / y),
            BinOp::Div if y != 0 => return float(x as f64 / y as f64),
            _ => None,
        };
        return r.map(Value::from).ok_or_else(|| ExprError(format!("integer overflow or division by zero: {x} {op:?} {y}")));
    }
    let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
    float(match op {
        BinOp::Add => x + y,
        BinOp::Sub => x - y,
        BinOp::Mul => x * y,
        BinOp::Div => x / y,
        _ => x % y,
    })
}

fn float(f: f64) -> Result<Value, ExprError> {
    Number::from_f64(f).map(Value::Number).ok_or_else(|| ExprError(format!("non-finite result {f}")))
}
