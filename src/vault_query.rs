//! Vault structured data query engine — Issue #1719.
//!
//! Provides an Obsidian-Bases-like structured query over vault notes based on
//! their frontmatter properties. The engine is pure and fully testable:
//!
//! - [`QValue`] — a typed scalar (Text / Number / Bool / Date / Null).
//! - [`Record`] — a single note: a path plus an arbitrary property map.
//! - [`Query`] — a parsed `SELECT … WHERE … ORDER BY … LIMIT …` statement.
//! - [`parse_query`] — a small recursive-descent parser for a SQL-ish dialect.
//! - [`query_records`] — executes a [`Query`] against a slice of [`Record`]s.
//!
//! ## Query syntax
//!
//! ```text
//! SELECT field1, field2
//! WHERE status = "active" AND priority >= 3
//! ORDER BY priority DESC
//! LIMIT 10
//! ```
//!
//! - `SELECT *` returns every property plus the synthetic `$path` column.
//! - Conditions are joined by `AND` / `OR` (with `AND` binding tighter) and may
//!   be grouped with parentheses. `NOT` negates a condition.
//! - Operators: `=`, `!=`, `<>`, `>`, `>=`, `<`, `<=`, `CONTAINS`, `IN`,
//!   `IS [NOT] NULL`.
//! - Values: numbers (`3`, `2.5`), double/single-quoted strings, `true`/`false`,
//!   ISO dates (`2026-07-13`), and for `IN` a parenthesized list.
//!
//! ## Bridging to real notes
//!
//! [`record_from_yaml`] converts a `serde_yaml_ng::Mapping` (as produced by the
//! existing frontmatter parser) into a [`Record`], so the engine can be wired to
//! the storage layer without coupling to its concrete `Frontmatter` struct.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use serde_yaml_ng::Value as Yaml;

/// A typed scalar value carried by a note property.
#[derive(Debug, Clone, PartialEq)]
pub enum QValue {
    Null,
    Text(String),
    Number(f64),
    Bool(bool),
    Date(NaiveDate),
}

impl QValue {
    /// Canonical ordering rank used when comparing values of different kinds
    /// (e.g. when sorting mixed-type columns).
    fn type_rank(&self) -> u8 {
        match self {
            QValue::Null => 0,
            QValue::Bool(_) => 1,
            QValue::Number(_) => 2,
            QValue::Date(_) => 3,
            QValue::Text(_) => 4,
        }
    }

    /// Render a stable, human-readable string for mixed-kind comparison/fallback.
    fn as_sort_key(&self) -> String {
        match self {
            QValue::Null => String::new(),
            QValue::Bool(b) => b.to_string(),
            QValue::Number(n) => format!("{n:.10}"),
            QValue::Date(d) => d.to_string(),
            QValue::Text(t) => t.clone(),
        }
    }
}

impl std::fmt::Display for QValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QValue::Null => write!(f, "null"),
            QValue::Bool(b) => write!(f, "{b}"),
            QValue::Number(n) => write!(f, "{n}"),
            QValue::Date(d) => write!(f, "{}", d.format("%Y-%m-%d")),
            QValue::Text(t) => write!(f, "{t}"),
        }
    }
}

/// A single vault note expressed as a property bag.
#[derive(Debug, Clone)]
pub struct Record {
    pub path: String,
    pub props: HashMap<String, QValue>,
}

impl Record {
    pub fn new(path: impl Into<String>) -> Self {
        Record {
            path: path.into(),
            props: HashMap::new(),
        }
    }

    pub fn with_prop(mut self, key: impl Into<String>, value: QValue) -> Self {
        self.props.insert(key.into(), value);
        self
    }
}

/// Sort direction for `ORDER BY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Asc,
    Desc,
}

/// A single boolean filter condition.
#[derive(Debug, Clone)]
pub enum Condition {
    Cmp {
        field: String,
        op: CmpOp,
        value: Operand,
    },
    IsNull {
        field: String,
        negated: bool,
    },
    In {
        field: String,
        values: Vec<Operand>,
    },
    Not(Box<Condition>),
    And(Box<Condition>, Box<Condition>),
    Or(Box<Condition>, Box<Condition>),
    /// Always-true placeholder used when a query has no `WHERE` clause.
    True,
}

/// Comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Contains,
}

/// A literal operand on the right-hand side of a condition.
#[derive(Debug, Clone)]
pub enum Operand {
    Literal(QValue),
    /// A bareword in value position that is *not* a literal (kept for forwards
    /// compatibility); treated as a quoted text value.
    Ident(String),
}

impl Operand {
    fn into_value(self) -> QValue {
        match self {
            Operand::Literal(v) => v,
            Operand::Ident(s) => QValue::Text(s),
        }
    }
}

/// A parsed query.
#[derive(Debug, Clone)]
pub struct Query {
    /// Selected field names, or `None` for `SELECT *`.
    pub select: Option<Vec<String>>,
    pub filter: Condition,
    pub order_by: Option<(String, Direction)>,
    pub limit: Option<usize>,
}

impl Default for Query {
    fn default() -> Self {
        Query {
            select: None,
            filter: Condition::True,
            order_by: None,
            limit: None,
        }
    }
}

// ── Tokenizer ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Number(f64),
    String_(String),
    Op(String), // one of = != <> > >= < <= ( )
    Comma,
    LParen,
    RParen,
    Kw(String), // SELECT WHERE ORDER BY ASC DESC LIMIT AND OR NOT IN IS NULL CONTAINS
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}
fn is_ident_cont(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

fn tokenize(src: &str) -> Result<Vec<Tok>> {
    let mut chars = src.chars().peekable();
    let mut toks = Vec::new();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '"' || c == '\'' {
            let quote = c;
            chars.next();
            let mut s = String::new();
            while let Some(ch) = chars.next() {
                if ch == quote {
                    break;
                }
                if ch == '\\' {
                    if let Some(next) = chars.next() {
                        s.push(next);
                    }
                    continue;
                }
                s.push(ch);
            }
            toks.push(Tok::String_(s));
            continue;
        }
        if c.is_ascii_digit()
            || (c == '.' && chars.clone().nth(1).is_some_and(|n| n.is_ascii_digit()))
        {
            let mut s = String::new();
            while let Some(&n) = chars.peek() {
                if n.is_ascii_digit() || n == '.' {
                    s.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            let num: f64 = s.parse().map_err(|_| anyhow!("invalid number: {s}"))?;
            toks.push(Tok::Number(num));
            continue;
        }
        if is_ident_start(c) {
            let mut s = String::new();
            while let Some(&n) = chars.peek() {
                if is_ident_cont(n) {
                    s.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            let upper = s.to_ascii_uppercase();
            match upper.as_str() {
                "SELECT" | "WHERE" | "ORDER" | "BY" | "ASC" | "DESC" | "LIMIT" | "AND" | "OR"
                | "NOT" | "IN" | "IS" | "NULL" | "CONTAINS" => toks.push(Tok::Kw(upper)),
                _ => toks.push(Tok::Ident(s)),
            }
            continue;
        }
        // operators / punctuation
        let two = src[src.find(c).unwrap()..]
            .chars()
            .take(2)
            .collect::<String>();
        match two.as_str() {
            ">=" | "<=" | "!=" | "<>" => {
                toks.push(Tok::Op(two));
                chars.next();
                chars.next();
            }
            _ => match c {
                '=' | '>' | '<' | '*' => {
                    toks.push(Tok::Op(c.to_string()));
                    chars.next();
                }
                '(' => {
                    toks.push(Tok::LParen);
                    chars.next();
                }
                ')' => {
                    toks.push(Tok::RParen);
                    chars.next();
                }
                ',' => {
                    toks.push(Tok::Comma);
                    chars.next();
                }
                other => return Err(anyhow!("unexpected character: {other}")),
            },
        }
    }
    Ok(toks)
}

// ── Parser ────────────────────────────────────────────────────────────────

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn new(toks: Vec<Tok>) -> Self {
        Parser { toks, pos: 0 }
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect_kw(&mut self, kw: &str) -> Result<()> {
        match self.next() {
            Some(Tok::Kw(k)) if k == kw => Ok(()),
            other => Err(anyhow!("expected {kw}, found {other:?}")),
        }
    }

    fn parse_query(&mut self) -> Result<Query> {
        self.expect_kw("SELECT")?;
        let select = self.parse_select()?;
        let mut filter = Condition::True;
        if matches!(self.peek(), Some(Tok::Kw(k)) if k == "WHERE") {
            self.next();
            filter = self.parse_expr()?;
        }
        let mut order_by = None;
        if matches!(self.peek(), Some(Tok::Kw(k)) if k == "ORDER") {
            self.next();
            self.expect_kw("BY")?;
            let field = match self.next() {
                Some(Tok::Ident(f)) => f,
                other => return Err(anyhow!("expected field after ORDER BY, found {other:?}")),
            };
            let dir = if matches!(self.peek(), Some(Tok::Kw(k)) if k == "ASC") {
                self.next();
                Direction::Asc
            } else if matches!(self.peek(), Some(Tok::Kw(k)) if k == "DESC") {
                self.next();
                Direction::Desc
            } else {
                Direction::Asc
            };
            order_by = Some((field, dir));
        }
        let mut limit = None;
        if matches!(self.peek(), Some(Tok::Kw(k)) if k == "LIMIT") {
            self.next();
            match self.next() {
                Some(Tok::Number(n)) if n >= 0.0 => limit = Some(n as usize),
                other => return Err(anyhow!("expected number after LIMIT, found {other:?}")),
            }
        }
        if self.pos != self.toks.len() {
            return Err(anyhow!(
                "unexpected trailing tokens at position {}",
                self.pos
            ));
        }
        Ok(Query {
            select,
            filter,
            order_by,
            limit,
        })
    }

    fn parse_select(&mut self) -> Result<Option<Vec<String>>> {
        if matches!(self.peek(), Some(Tok::Op(o)) if o == "*") {
            self.next();
            return Ok(None);
        }
        let mut fields = Vec::new();
        loop {
            match self.next() {
                Some(Tok::Ident(f)) => fields.push(f),
                other => return Err(anyhow!("expected field name in SELECT, found {other:?}")),
            }
            if matches!(self.peek(), Some(Tok::Comma)) {
                self.next();
                continue;
            }
            break;
        }
        Ok(Some(fields))
    }

    fn parse_expr(&mut self) -> Result<Condition> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Condition> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(Tok::Kw(k)) if k == "OR") {
            self.next();
            let right = self.parse_and()?;
            left = Condition::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Condition> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), Some(Tok::Kw(k)) if k == "AND") {
            self.next();
            let right = self.parse_not()?;
            left = Condition::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Condition> {
        if matches!(self.peek(), Some(Tok::Kw(k)) if k == "NOT") {
            self.next();
            let inner = self.parse_not()?;
            return Ok(Condition::Not(Box::new(inner)));
        }
        self.parse_term()
    }

    fn parse_term(&mut self) -> Result<Condition> {
        if matches!(self.peek(), Some(Tok::LParen)) {
            self.next();
            let inner = self.parse_expr()?;
            match self.next() {
                Some(Tok::RParen) => Ok(inner),
                other => Err(anyhow!("expected ) , found {other:?}")),
            }
        } else {
            self.parse_condition()
        }
    }

    fn parse_condition(&mut self) -> Result<Condition> {
        let field = match self.next() {
            Some(Tok::Ident(f)) => f,
            other => return Err(anyhow!("expected field name, found {other:?}")),
        };
        // IS [NOT] NULL
        if matches!(self.peek(), Some(Tok::Kw(k)) if k == "IS") {
            self.next();
            let negated = if matches!(self.peek(), Some(Tok::Kw(k)) if k == "NOT") {
                self.next();
                true
            } else {
                false
            };
            self.expect_kw("NULL")?;
            return Ok(Condition::IsNull { field, negated });
        }
        // IN ( ... )
        if matches!(self.peek(), Some(Tok::Kw(k)) if k == "IN") {
            self.next();
            match self.next() {
                Some(Tok::LParen) => {}
                other => return Err(anyhow!("expected ( after IN, found {other:?}")),
            }
            let mut values = Vec::new();
            loop {
                values.push(self.parse_operand()?);
                if matches!(self.peek(), Some(Tok::Comma)) {
                    self.next();
                    continue;
                }
                break;
            }
            match self.next() {
                Some(Tok::RParen) => {}
                other => return Err(anyhow!("expected ) to close IN list, found {other:?}")),
            }
            return Ok(Condition::In { field, values });
        }
        // comparison operator
        let op = match self.next() {
            Some(Tok::Kw(k)) if k == "CONTAINS" => CmpOp::Contains,
            Some(Tok::Op(o)) => match o.as_str() {
                "=" => CmpOp::Eq,
                "!=" | "<>" => CmpOp::Ne,
                ">" => CmpOp::Gt,
                ">=" => CmpOp::Ge,
                "<" => CmpOp::Lt,
                "<=" => CmpOp::Le,
                other => return Err(anyhow!("unsupported operator: {other}")),
            },
            other => return Err(anyhow!("expected operator, found {other:?}")),
        };
        let value = self.parse_operand()?;
        Ok(Condition::Cmp { field, op, value })
    }

    fn parse_operand(&mut self) -> Result<Operand> {
        match self.next() {
            Some(Tok::Number(n)) => Ok(Operand::Literal(QValue::Number(n))),
            Some(Tok::String_(s)) => Ok(Operand::Literal(QValue::Text(s))),
            Some(Tok::Ident(s)) => {
                let upper = s.to_ascii_uppercase();
                if upper == "TRUE" {
                    return Ok(Operand::Literal(QValue::Bool(true)));
                }
                if upper == "FALSE" {
                    return Ok(Operand::Literal(QValue::Bool(false)));
                }
                if let Ok(d) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
                    return Ok(Operand::Literal(QValue::Date(d)));
                }
                Ok(Operand::Ident(s))
            }
            other => Err(anyhow!("expected value, found {other:?}")),
        }
    }
}

/// Parse a query string into a [`Query`].
pub fn parse_query(src: &str) -> Result<Query> {
    let toks = tokenize(src)?;
    if toks.is_empty() {
        return Err(anyhow!("empty query"));
    }
    let mut parser = Parser::new(toks);
    parser.parse_query()
}

// ── Evaluation ─────────────────────────────────────────────────────────────

fn get_prop(rec: &Record, field: &str) -> QValue {
    rec.props.get(field).cloned().unwrap_or(QValue::Null)
}

/// Coerce two values to a comparable pair for equality / ordering.
fn coerce_eq(a: &QValue, b: &QValue) -> bool {
    match (a, b) {
        (QValue::Null, QValue::Null) => true,
        (QValue::Null, _) | (_, QValue::Null) => false,
        (QValue::Bool(x), QValue::Bool(y)) => x == y,
        (QValue::Number(x), QValue::Number(y)) => x == y,
        (QValue::Date(x), QValue::Date(y)) => x == y,
        (QValue::Text(x), QValue::Text(y)) => x == y,
        // Number vs numeric Text: compare numerically for usability.
        (QValue::Number(x), QValue::Text(y)) | (QValue::Text(y), QValue::Number(x)) => y
            .parse::<f64>()
            .map(|fy| (*x - fy).abs() < f64::EPSILON)
            .unwrap_or(false),
        _ => a.as_sort_key() == b.as_sort_key(),
    }
}

fn cmp_ord(a: &QValue, b: &QValue) -> std::cmp::Ordering {
    if a.type_rank() != b.type_rank() {
        return a.type_rank().cmp(&b.type_rank());
    }
    match (a, b) {
        (QValue::Number(x), QValue::Number(y)) => {
            x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
        }
        (QValue::Date(x), QValue::Date(y)) => x.cmp(y),
        (QValue::Bool(x), QValue::Bool(y)) => x.cmp(y),
        (QValue::Text(x), QValue::Text(y)) => x.cmp(y),
        (QValue::Null, QValue::Null) => std::cmp::Ordering::Equal,
        _ => a.as_sort_key().cmp(&b.as_sort_key()),
    }
}

fn eval_cond(cond: &Condition, rec: &Record) -> bool {
    match cond {
        Condition::Cmp { field, op, value } => {
            let left = get_prop(rec, field);
            let right = value.clone().into_value();
            match op {
                CmpOp::Eq => coerce_eq(&left, &right),
                CmpOp::Ne => !coerce_eq(&left, &right),
                CmpOp::Contains => match (&left, &right) {
                    (QValue::Text(l), QValue::Text(r)) => {
                        l.to_lowercase().contains(&r.to_lowercase())
                    }
                    _ => false,
                },
                CmpOp::Gt => cmp_ord(&left, &right) == std::cmp::Ordering::Greater,
                CmpOp::Ge => cmp_ord(&left, &right) != std::cmp::Ordering::Less,
                CmpOp::Lt => cmp_ord(&left, &right) == std::cmp::Ordering::Less,
                CmpOp::Le => cmp_ord(&left, &right) != std::cmp::Ordering::Greater,
            }
        }
        Condition::IsNull { field, negated } => {
            let is_null = matches!(get_prop(rec, field), QValue::Null);
            if *negated {
                !is_null
            } else {
                is_null
            }
        }
        Condition::In { field, values } => {
            let left = get_prop(rec, field);
            values
                .iter()
                .any(|v| coerce_eq(&left, &v.clone().into_value()))
        }
        Condition::Not(inner) => !eval_cond(inner, rec),
        Condition::And(l, r) => eval_cond(l, rec) && eval_cond(r, rec),
        Condition::Or(l, r) => eval_cond(l, rec) || eval_cond(r, rec),
        Condition::True => true,
    }
}

/// Execute `query` against `records`, returning projected rows.
///
/// Each returned row is a property map. The synthetic `$path` column is always
/// included so callers can locate the source note.
pub fn query_records(records: &[Record], query: &Query) -> Vec<HashMap<String, QValue>> {
    let mut matched: Vec<&Record> = records
        .iter()
        .filter(|r| eval_cond(&query.filter, r))
        .collect();

    if let Some((field, dir)) = &query.order_by {
        matched.sort_by(|a, b| {
            let c = cmp_ord(&get_prop(a, field), &get_prop(b, field));
            match dir {
                Direction::Asc => c,
                Direction::Desc => c.reverse(),
            }
        });
    }

    if let Some(limit) = query.limit {
        matched.truncate(limit);
    }

    matched
        .into_iter()
        .map(|r| {
            let mut row: HashMap<String, QValue> = HashMap::new();
            row.insert("$path".to_string(), QValue::Text(r.path.clone()));
            match &query.select {
                None => {
                    for (k, v) in &r.props {
                        row.insert(k.clone(), v.clone());
                    }
                }
                Some(fields) => {
                    for f in fields {
                        row.insert(f.clone(), get_prop(r, f));
                    }
                }
            }
            row
        })
        .collect()
}

// ── Bridging helpers ───────────────────────────────────────────────────────

fn yaml_to_qvalue(v: &Yaml) -> QValue {
    match v {
        Yaml::Null => QValue::Null,
        Yaml::Bool(b) => QValue::Bool(*b),
        Yaml::Number(n) => QValue::Number(n.as_f64().unwrap_or(f64::NAN)),
        Yaml::String(s) => {
            // Try ISO date first for nicer typing.
            if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                QValue::Date(d)
            } else {
                QValue::Text(s.clone())
            }
        }
        // Sequences / mappings are stored as Text for now (Bases relations/lists
        // can be refined later without breaking the engine).
        other => QValue::Text(serde_yaml_ng::to_string(other).unwrap_or_default()),
    }
}

/// Convert a `serde_yaml_ng::Mapping` (frontmatter) into a [`Record`].
pub fn record_from_yaml(path: &str, mapping: &serde_yaml_ng::Mapping) -> Record {
    let mut rec = Record::new(path);
    for (k, v) in mapping {
        if let Yaml::String(key) = k {
            rec.props.insert(key.clone(), yaml_to_qvalue(v));
        }
    }
    rec
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(path: &str) -> Record {
        Record::new(path)
    }

    fn sample_records() -> Vec<Record> {
        vec![
            rec("a.md")
                .with_prop("status", QValue::Text("active".into()))
                .with_prop("priority", QValue::Number(3.0))
                .with_prop("project", QValue::Text("vault".into()))
                .with_prop(
                    "due",
                    QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
                ),
            rec("b.md")
                .with_prop("status", QValue::Text("done".into()))
                .with_prop("priority", QValue::Number(1.0))
                .with_prop("project", QValue::Text("vault".into()))
                .with_prop(
                    "due",
                    QValue::Date(NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()),
                ),
            rec("c.md")
                .with_prop("status", QValue::Text("active".into()))
                .with_prop("priority", QValue::Number(5.0))
                .with_prop("project", QValue::Text("mobile".into()))
                .with_prop(
                    "due",
                    QValue::Date(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()),
                ),
            rec("d.md").with_prop("status", QValue::Text("backlog".into())),
        ]
    }

    #[test]
    fn parses_select_star_and_where_eq() {
        let q = parse_query(r#"SELECT * WHERE status = "active""#).unwrap();
        let rows = query_records(&sample_records(), &q);
        assert_eq!(rows.len(), 2);
        for r in &rows {
            assert_eq!(r.get("status"), Some(&QValue::Text("active".into())));
            assert!(r.contains_key("$path"));
        }
    }

    #[test]
    fn parses_projection() {
        let q = parse_query(r#"SELECT status, priority WHERE project = "vault""#).unwrap();
        let rows = query_records(&sample_records(), &q);
        assert_eq!(rows.len(), 2);
        for r in &rows {
            assert!(r.contains_key("status"));
            assert!(r.contains_key("priority"));
            assert!(!r.contains_key("project"));
        }
    }

    #[test]
    fn numeric_comparison_and_sort() {
        let q = parse_query("SELECT * WHERE priority >= 3 ORDER BY priority DESC").unwrap();
        let rows = query_records(&sample_records(), &q);
        assert_eq!(rows.len(), 2);
        // c (5) then a (3)
        let paths: Vec<&QValue> = rows.iter().map(|r| r.get("$path").unwrap()).collect();
        assert_eq!(paths[0], &QValue::Text("c.md".into()));
        assert_eq!(paths[1], &QValue::Text("a.md".into()));
    }

    #[test]
    fn and_or_precedence() {
        // AND binds tighter than OR.
        let q = parse_query(
            r#"SELECT * WHERE status = "active" OR (priority >= 3 AND project = "vault")"#,
        )
        .unwrap();
        let rows = query_records(&sample_records(), &q);
        // active: a, c ; vault>=3: a  => a, c
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn not_and_is_null() {
        let q = parse_query(r#"SELECT * WHERE NOT status = "active""#).unwrap();
        let rows = query_records(&sample_records(), &q);
        assert_eq!(rows.len(), 2); // b (done), d (backlog)

        let q2 = parse_query("SELECT * WHERE priority IS NOT NULL").unwrap();
        let rows2 = query_records(&sample_records(), &q2);
        assert_eq!(rows2.len(), 3);
    }

    #[test]
    fn contains_operator_case_insensitive() {
        let q = parse_query(r#"SELECT * WHERE project CONTAINS "MOB""#).unwrap();
        let rows = query_records(&sample_records(), &q);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("$path"), Some(&QValue::Text("c.md".into())));
    }

    #[test]
    fn in_operator() {
        let q = parse_query(r#"SELECT * WHERE status IN ("done", "backlog")"#).unwrap();
        let rows = query_records(&sample_records(), &q);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn limit_clause() {
        let q = parse_query("SELECT * ORDER BY priority DESC LIMIT 1").unwrap();
        let rows = query_records(&sample_records(), &q);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("$path"), Some(&QValue::Text("c.md".into())));
    }

    #[test]
    fn record_from_yaml_typing() {
        let mut m = serde_yaml_ng::Mapping::new();
        m.insert(Yaml::String("status".into()), Yaml::String("active".into()));
        m.insert(Yaml::String("priority".into()), Yaml::Number(4.into()));
        m.insert(
            Yaml::String("due".into()),
            Yaml::String("2026-07-13".into()),
        );
        let rec = record_from_yaml("x.md", &m);
        assert_eq!(
            rec.props.get("status"),
            Some(&QValue::Text("active".into()))
        );
        assert_eq!(rec.props.get("priority"), Some(&QValue::Number(4.0)));
        assert_eq!(
            rec.props.get("due"),
            Some(&QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 13).unwrap()))
        );
    }

    #[test]
    fn rejects_unknown_trailing_tokens() {
        assert!(parse_query(r#"SELECT * WHERE status = "x" GARBAGE"#).is_err());
    }
}
