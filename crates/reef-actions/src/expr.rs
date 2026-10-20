//! The `${{ }}` expression language: literals, context lookups (`github.ref`,
//! `needs.build.outputs.x`, `matrix.os`, `needs.*.result`), the operators `! < <= > >= == !=
//! && ||`, grouping, and the functions `contains startsWith endsWith format join toJSON
//! fromJSON hashFiles success always cancelled failure`. Values are JSON values.

use crate::{Error, Result};
pub use serde_json::Value;
use serde_json::json;

/// Job status the status functions report on. For a job-level `if`, this describes the
/// dependencies (`needs`); for a workflow-level condition it describes the run so far.
#[derive(Debug, Clone, Copy, Default)]
pub struct Status {
    pub success: bool,
    pub failure: bool,
    pub cancelled: bool,
}

impl Status {
    pub fn all_ok() -> Status {
        Status { success: true, failure: false, cancelled: false }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Dot,
    Star,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Comma,
    Not,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

fn lex(s: &str) -> Result<Vec<Tok>> {
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < cs.len() {
        let c = cs[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '.' if !(i + 1 < cs.len()
                && cs[i + 1].is_ascii_digit()
                && !matches!(out.last(), Some(Tok::Ident(_)) | Some(Tok::RBracket) | Some(Tok::Star))) =>
            {
                out.push(Tok::Dot);
                i += 1
            }
            '*' => {
                out.push(Tok::Star);
                i += 1
            }
            '[' => {
                out.push(Tok::LBracket);
                i += 1
            }
            ']' => {
                out.push(Tok::RBracket);
                i += 1
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1
            }
            '!' if i + 1 < cs.len() && cs[i + 1] == '=' => {
                out.push(Tok::Ne);
                i += 2
            }
            '!' => {
                out.push(Tok::Not);
                i += 1
            }
            '<' if i + 1 < cs.len() && cs[i + 1] == '=' => {
                out.push(Tok::Le);
                i += 2
            }
            '<' => {
                out.push(Tok::Lt);
                i += 1
            }
            '>' if i + 1 < cs.len() && cs[i + 1] == '=' => {
                out.push(Tok::Ge);
                i += 2
            }
            '>' => {
                out.push(Tok::Gt);
                i += 1
            }
            '=' if i + 1 < cs.len() && cs[i + 1] == '=' => {
                out.push(Tok::Eq);
                i += 2
            }
            '&' if i + 1 < cs.len() && cs[i + 1] == '&' => {
                out.push(Tok::And);
                i += 2
            }
            '|' if i + 1 < cs.len() && cs[i + 1] == '|' => {
                out.push(Tok::Or);
                i += 2
            }
            '\'' => {
                let mut v = String::new();
                i += 1;
                loop {
                    if i >= cs.len() {
                        return Err(Error::Expr("unterminated string".into()));
                    }
                    if cs[i] == '\'' {
                        if i + 1 < cs.len() && cs[i + 1] == '\'' {
                            v.push('\'');
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    v.push(cs[i]);
                    i += 1;
                }
                out.push(Tok::Str(v));
            }
            c if c.is_ascii_digit()
                || ((c == '-' || c == '+' || c == '.') && i + 1 < cs.len() && (cs[i + 1].is_ascii_digit() || cs[i + 1] == '.')) =>
            {
                let start = i;
                i += 1;
                if c == '0' && i < cs.len() && (cs[i] == 'x' || cs[i] == 'X') {
                    i += 1;
                    while i < cs.len() && cs[i].is_ascii_hexdigit() {
                        i += 1;
                    }
                    let n =
                        i64::from_str_radix(&cs[start + 2..i].iter().collect::<String>(), 16).map_err(|e| Error::Expr(e.to_string()))?;
                    out.push(Tok::Num(n as f64));
                    continue;
                }
                while i < cs.len()
                    && (cs[i].is_ascii_digit()
                        || cs[i] == '.'
                        || cs[i] == 'e'
                        || cs[i] == 'E'
                        || ((cs[i] == '-' || cs[i] == '+') && matches!(cs[i - 1], 'e' | 'E')))
                {
                    i += 1;
                }
                let text: String = cs[start..i].iter().collect();
                out.push(Tok::Num(text.parse::<f64>().map_err(|_| Error::Expr(format!("bad number '{text}'")))?));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < cs.len() && (cs[i].is_ascii_alphanumeric() || cs[i] == '_' || cs[i] == '-') {
                    i += 1;
                }
                out.push(Tok::Ident(cs[start..i].iter().collect()));
            }
            other => return Err(Error::Expr(format!("unexpected character '{other}'"))),
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
enum Node {
    Lit(Value),
    Ctx(String),
    Prop(Box<Node>, String),
    Index(Box<Node>, Box<Node>),
    Wild(Box<Node>),
    Call(String, Vec<Node>),
    Not(Box<Node>),
    Bin(Tok, Box<Node>, Box<Node>),
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn expect(&mut self, t: Tok) -> Result<()> {
        match self.next() {
            Some(ref x) if *x == t => Ok(()),
            other => Err(Error::Expr(format!("expected {t:?}, found {other:?}"))),
        }
    }
    fn or(&mut self) -> Result<Node> {
        let mut l = self.and()?;
        while self.peek() == Some(&Tok::Or) {
            self.next();
            let r = self.and()?;
            l = Node::Bin(Tok::Or, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Node> {
        let mut l = self.cmp()?;
        while self.peek() == Some(&Tok::And) {
            self.next();
            let r = self.cmp()?;
            l = Node::Bin(Tok::And, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn cmp(&mut self) -> Result<Node> {
        let mut l = self.unary()?;
        while let Some(op @ (Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge | Tok::Eq | Tok::Ne)) = self.peek().cloned() {
            self.next();
            let r = self.unary()?;
            l = Node::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Node> {
        if self.peek() == Some(&Tok::Not) {
            self.next();
            return Ok(Node::Not(Box::new(self.unary()?)));
        }
        self.postfix()
    }
    fn postfix(&mut self) -> Result<Node> {
        let mut n = self.primary()?;
        loop {
            match self.peek() {
                Some(Tok::Dot) => {
                    self.next();
                    match self.next() {
                        Some(Tok::Ident(name)) => n = Node::Prop(Box::new(n), name),
                        Some(Tok::Star) => n = Node::Wild(Box::new(n)),
                        other => return Err(Error::Expr(format!("expected property name, found {other:?}"))),
                    }
                }
                Some(Tok::LBracket) => {
                    self.next();
                    if self.peek() == Some(&Tok::Star) {
                        self.next();
                        self.expect(Tok::RBracket)?;
                        n = Node::Wild(Box::new(n));
                    } else {
                        let idx = self.or()?;
                        self.expect(Tok::RBracket)?;
                        n = Node::Index(Box::new(n), Box::new(idx));
                    }
                }
                _ => return Ok(n),
            }
        }
    }
    fn primary(&mut self) -> Result<Node> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Node::Lit(json!(n))),
            Some(Tok::Str(s)) => Ok(Node::Lit(Value::String(s))),
            Some(Tok::LParen) => {
                let e = self.or()?;
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            Some(Tok::Ident(id)) => {
                if self.peek() == Some(&Tok::LParen) {
                    self.next();
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::RParen) {
                        loop {
                            args.push(self.or()?);
                            if self.peek() == Some(&Tok::Comma) {
                                self.next();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Tok::RParen)?;
                    return Ok(Node::Call(id.to_lowercase(), args));
                }
                match id.to_lowercase().as_str() {
                    "true" => Ok(Node::Lit(Value::Bool(true))),
                    "false" => Ok(Node::Lit(Value::Bool(false))),
                    "null" => Ok(Node::Lit(Value::Null)),
                    "nan" => Ok(Node::Lit(Value::Null)),
                    "infinity" => Ok(Node::Lit(json!(f64::MAX))),
                    _ => Ok(Node::Ctx(id)),
                }
            }
            other => Err(Error::Expr(format!("unexpected token {other:?}"))),
        }
    }
}

fn parse(s: &str) -> Result<Node> {
    let toks = lex(s)?;
    let mut p = Parser { toks, pos: 0 };
    let n = p.or()?;
    if p.pos != p.toks.len() {
        return Err(Error::Expr(format!("unexpected trailing input in '{s}'")));
    }
    Ok(n)
}

/// Coerce to a number the way GitHub does: null→0, bool→0/1, numeric strings parse, ''→0,
/// anything else is NaN.
pub fn to_number(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                i64::from_str_radix(h, 16).map(|n| n as f64).unwrap_or(f64::NAN)
            } else {
                t.parse::<f64>().unwrap_or(f64::NAN)
            }
        }
        _ => f64::NAN,
    }
}

pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// Render a value as GitHub does when interpolating into a string.
pub fn to_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            if f.fract() == 0.0 && f.abs() < 1e15 { format!("{}", f as i64) } else { f.to_string() }
        }
        Value::String(s) => s.clone(),
        Value::Array(_) => "Array".into(),
        Value::Object(_) => "Object".into(),
    }
}

fn loose_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.eq_ignore_ascii_case(y),
        (Value::Array(_) | Value::Object(_), _) | (_, Value::Array(_) | Value::Object(_)) => std::ptr::eq(a, b),
        _ => {
            let (x, y) = (to_number(a), to_number(b));
            !x.is_nan() && !y.is_nan() && x == y
        }
    }
}

fn compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::String(x), Value::String(y)) => Some(x.to_lowercase().cmp(&y.to_lowercase())),
        _ => {
            let (x, y) = (to_number(a), to_number(b));
            x.partial_cmp(&y)
        }
    }
}

fn get(v: &Value, key: &str) -> Value {
    match v {
        Value::Object(m) => m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone()).unwrap_or(Value::Null),
        Value::Array(items) => Value::Array(items.iter().map(|i| get(i, key)).filter(|v| !v.is_null()).collect()),
        _ => Value::Null,
    }
}

fn format_fn(args: &[Value]) -> Result<Value> {
    let Some(Value::String(fmt)) = args.first() else { return Err(Error::Expr("format() needs a string".into())) };
    let mut out = String::new();
    let cs: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '{' if i + 1 < cs.len() && cs[i + 1] == '{' => {
                out.push('{');
                i += 2;
            }
            '}' if i + 1 < cs.len() && cs[i + 1] == '}' => {
                out.push('}');
                i += 2;
            }
            '{' => {
                let Some(end) = cs[i..].iter().position(|&c| c == '}') else { return Err(Error::Expr("format(): unclosed '{'".into())) };
                let idx: usize = cs[i + 1..i + end]
                    .iter()
                    .collect::<String>()
                    .trim()
                    .parse()
                    .map_err(|_| Error::Expr("format(): bad placeholder".into()))?;
                let v = args.get(idx + 1).ok_or_else(|| Error::Expr(format!("format(): missing argument {idx}")))?;
                out.push_str(&to_string(v));
                i += end + 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(Value::String(out))
}

fn eval(n: &Node, ctx: &Value, status: Status) -> Result<Value> {
    Ok(match n {
        Node::Lit(v) => v.clone(),
        Node::Ctx(name) => get(ctx, name),
        Node::Prop(obj, key) => get(&eval(obj, ctx, status)?, key),
        Node::Index(obj, idx) => {
            let o = eval(obj, ctx, status)?;
            let i = eval(idx, ctx, status)?;
            match (&o, &i) {
                (Value::Array(items), Value::Number(_)) => {
                    let k = to_number(&i);
                    if k.is_nan() || k < 0.0 { Value::Null } else { items.get(k as usize).cloned().unwrap_or(Value::Null) }
                }
                (Value::Object(_), Value::String(s)) => get(&o, s),
                (Value::Array(_), Value::String(s)) => get(&o, s),
                (Value::Object(_), Value::Number(_)) => get(&o, &to_string(&i)),
                _ => Value::Null,
            }
        }
        Node::Wild(obj) => match eval(obj, ctx, status)? {
            Value::Object(m) => Value::Array(m.into_iter().map(|(_, v)| v).collect()),
            Value::Array(a) => Value::Array(a),
            _ => Value::Array(vec![]),
        },
        Node::Not(e) => Value::Bool(!truthy(&eval(e, ctx, status)?)),
        Node::Bin(op, l, r) => {
            let a = eval(l, ctx, status)?;
            match op {
                Tok::And => {
                    if truthy(&a) {
                        eval(r, ctx, status)?
                    } else {
                        a
                    }
                }
                Tok::Or => {
                    if truthy(&a) {
                        a
                    } else {
                        eval(r, ctx, status)?
                    }
                }
                _ => {
                    let b = eval(r, ctx, status)?;
                    Value::Bool(match op {
                        Tok::Eq => loose_eq(&a, &b),
                        Tok::Ne => !loose_eq(&a, &b),
                        Tok::Lt => compare(&a, &b) == Some(std::cmp::Ordering::Less),
                        Tok::Le => matches!(compare(&a, &b), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)),
                        Tok::Gt => compare(&a, &b) == Some(std::cmp::Ordering::Greater),
                        Tok::Ge => matches!(compare(&a, &b), Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)),
                        _ => unreachable!(),
                    })
                }
            }
        }
        Node::Call(name, args) => {
            let vals: Vec<Value> = args.iter().map(|a| eval(a, ctx, status)).collect::<Result<_>>()?;
            match name.as_str() {
                "success" => Value::Bool(status.success),
                "failure" => Value::Bool(status.failure),
                "cancelled" => Value::Bool(status.cancelled),
                "always" => Value::Bool(true),
                "contains" => {
                    let (Some(hay), Some(needle)) = (vals.first(), vals.get(1)) else {
                        return Err(Error::Expr("contains() needs 2 arguments".into()));
                    };
                    Value::Bool(match hay {
                        Value::Array(items) => items.iter().any(|i| loose_eq(i, needle)),
                        other => to_string(other).to_lowercase().contains(&to_string(needle).to_lowercase()),
                    })
                }
                "startswith" => {
                    let (Some(s), Some(p)) = (vals.first(), vals.get(1)) else {
                        return Err(Error::Expr("startsWith() needs 2 arguments".into()));
                    };
                    Value::Bool(to_string(s).to_lowercase().starts_with(&to_string(p).to_lowercase()))
                }
                "endswith" => {
                    let (Some(s), Some(p)) = (vals.first(), vals.get(1)) else {
                        return Err(Error::Expr("endsWith() needs 2 arguments".into()));
                    };
                    Value::Bool(to_string(s).to_lowercase().ends_with(&to_string(p).to_lowercase()))
                }
                "format" => format_fn(&vals)?,
                "join" => {
                    let sep = vals.get(1).map(to_string).unwrap_or_else(|| ",".into());
                    match vals.first() {
                        Some(Value::Array(items)) => Value::String(items.iter().map(to_string).collect::<Vec<_>>().join(&sep)),
                        Some(other) => Value::String(to_string(other)),
                        None => return Err(Error::Expr("join() needs an argument".into())),
                    }
                }
                "tojson" => Value::String(serde_json::to_string_pretty(vals.first().unwrap_or(&Value::Null)).unwrap_or_default()),
                "fromjson" => {
                    let s = vals.first().map(to_string).unwrap_or_default();
                    if s.trim().is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_str(&s).map_err(|e| Error::Expr(format!("fromJSON: {e}")))?
                    }
                }
                // hashFiles needs the checkout; on the server it evaluates to the empty string.
                "hashfiles" => Value::String(String::new()),
                other => return Err(Error::Expr(format!("unknown function '{other}'"))),
            }
        }
    })
}

/// Evaluate a bare expression (no `${{ }}` delimiters).
pub fn evaluate(expr: &str, ctx: &Value, status: Status) -> Result<Value> {
    eval(&parse(expr)?, ctx, status)
}

/// Evaluate a job-level `if`: an expression with or without `${{ }}`. Like GitHub, a condition
/// that names no status function is implicitly `success() && (...)`.
pub fn condition(cond: &str, ctx: &Value, status: Status) -> Result<bool> {
    let raw = cond.trim();
    let inner = raw.strip_prefix("${{").and_then(|s| s.strip_suffix("}}")).map(|s| s.trim()).unwrap_or(raw);
    if inner.is_empty() {
        return Ok(status.success);
    }
    let node = parse(inner)?;
    let uses_status = mentions_status(&node);
    let v = truthy(&eval(&node, ctx, status)?);
    Ok(if uses_status { v } else { status.success && v })
}

fn mentions_status(n: &Node) -> bool {
    match n {
        Node::Call(name, args) => {
            matches!(name.as_str(), "success" | "failure" | "cancelled" | "always") || args.iter().any(mentions_status)
        }
        Node::Prop(o, _) | Node::Wild(o) | Node::Not(o) => mentions_status(o),
        Node::Index(a, b) | Node::Bin(_, a, b) => mentions_status(a) || mentions_status(b),
        _ => false,
    }
}

/// Does the string contain any `${{ }}`?
pub fn has_expression(s: &str) -> bool {
    s.contains("${{")
}

/// Replace every `${{ expr }}` in `s` with its rendered value.
pub fn interpolate(s: &str, ctx: &Value, status: Status) -> Result<String> {
    if !has_expression(s) {
        return Ok(s.to_string());
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("${{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 3..];
        let Some(end) = after.find("}}") else { return Err(Error::Expr("unclosed '${{'".into())) };
        let v = evaluate(after[..end].trim(), ctx, status)?;
        out.push_str(&to_string(&v));
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Evaluate a value that is either a single whole expression (which may yield an object or
/// array, e.g. `${{ fromJSON(...) }}`) or a string with embedded expressions.
pub fn evaluate_value(v: &Value, ctx: &Value, status: Status) -> Result<Value> {
    match v {
        Value::String(s) => {
            let t = s.trim();
            if let Some(inner) = t.strip_prefix("${{").and_then(|x| x.strip_suffix("}}"))
                && !inner.contains("${{")
            {
                return evaluate(inner.trim(), ctx, status);
            }
            Ok(Value::String(interpolate(s, ctx, status)?))
        }
        Value::Array(items) => Ok(Value::Array(items.iter().map(|i| evaluate_value(i, ctx, status)).collect::<Result<_>>()?)),
        Value::Object(m) => {
            let mut out = serde_json::Map::new();
            for (k, v) in m {
                out.insert(k.clone(), evaluate_value(v, ctx, status)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Value {
        json!({
            "github": {"ref": "refs/heads/main", "event_name": "push", "repository": "yadvlabs/reef", "event": {"head_commit": {"message": "fix: x [skip ci]"}}},
            "matrix": {"os": "ubuntu-latest", "node": 20},
            "needs": {"build": {"result": "success", "outputs": {"version": "1.2.3"}}, "lint": {"result": "skipped"}},
            "inputs": {"debug": true}
        })
    }

    #[test]
    fn literals_and_ops() {
        let c = ctx();
        let s = Status::all_ok();
        assert_eq!(evaluate("1 == 1.0", &c, s).unwrap(), json!(true));
        assert_eq!(evaluate("'ABC' == 'abc'", &c, s).unwrap(), json!(true));
        assert_eq!(evaluate("null == 0", &c, s).unwrap(), json!(true));
        assert_eq!(evaluate("!false && (1 < 2)", &c, s).unwrap(), json!(true));
        assert_eq!(evaluate("'a' || 'b'", &c, s).unwrap(), json!("a"));
        assert_eq!(evaluate("'' || 'b'", &c, s).unwrap(), json!("b"));
        assert_eq!(evaluate("0x10", &c, s).unwrap(), json!(16.0));
    }

    #[test]
    fn contexts() {
        let c = ctx();
        let s = Status::all_ok();
        assert_eq!(evaluate("github.ref == 'refs/heads/main'", &c, s).unwrap(), json!(true));
        assert_eq!(evaluate("matrix.node", &c, s).unwrap(), json!(20));
        assert_eq!(evaluate("needs.build.outputs.version", &c, s).unwrap(), json!("1.2.3"));
        assert_eq!(evaluate("needs['build'].result", &c, s).unwrap(), json!("success"));
        assert_eq!(evaluate("contains(needs.*.result, 'skipped')", &c, s).unwrap(), json!(true));
        assert_eq!(evaluate("contains(github.event.head_commit.message, '[skip ci]')", &c, s).unwrap(), json!(true));
        assert_eq!(evaluate("github.missing.deep", &c, s).unwrap(), Value::Null);
        assert_eq!(evaluate("inputs.debug", &c, s).unwrap(), json!(true));
    }

    #[test]
    fn functions() {
        let c = ctx();
        let s = Status::all_ok();
        assert_eq!(evaluate("startsWith(github.ref, 'refs/heads/')", &c, s).unwrap(), json!(true));
        assert_eq!(evaluate("endsWith('Hello', 'LO')", &c, s).unwrap(), json!(true));
        assert_eq!(evaluate("format('{0}-{1} {{x}}', matrix.os, matrix.node)", &c, s).unwrap(), json!("ubuntu-latest-20 {x}"));
        assert_eq!(evaluate("join(fromJSON('[\"a\",\"b\"]'), '+')", &c, s).unwrap(), json!("a+b"));
        assert_eq!(evaluate("fromJSON('{\"a\":1}').a", &c, s).unwrap(), json!(1));
        assert_eq!(evaluate("toJSON(matrix.node)", &c, s).unwrap(), json!("20"));
    }

    #[test]
    fn conditions() {
        let c = ctx();
        let ok = Status::all_ok();
        let failed = Status { success: false, failure: true, cancelled: false };
        assert!(condition("github.ref == 'refs/heads/main'", &c, ok).unwrap());
        assert!(!condition("github.ref == 'refs/heads/main'", &c, failed).unwrap());
        assert!(condition("${{ always() }}", &c, failed).unwrap());
        assert!(condition("failure()", &c, failed).unwrap());
        assert!(condition("", &c, ok).unwrap());
        assert!(!condition("needs.lint.result == 'success'", &c, ok).unwrap());
    }

    #[test]
    fn interpolation() {
        let c = ctx();
        let s = Status::all_ok();
        assert_eq!(interpolate("build-${{ matrix.os }}-${{ matrix.node }}", &c, s).unwrap(), "build-ubuntu-latest-20");
        assert_eq!(interpolate("plain", &c, s).unwrap(), "plain");
        assert_eq!(evaluate_value(&json!("${{ fromJSON('[1,2]') }}"), &c, s).unwrap(), json!([1, 2]));
        assert_eq!(evaluate_value(&json!(["${{ matrix.os }}", "self-hosted"]), &c, s).unwrap(), json!(["ubuntu-latest", "self-hosted"]));
    }
}
