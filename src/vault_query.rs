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
    /// A YAML sequence (list) value — used for frontmatter array properties
    /// like `tags: [rust, ai, notes]`.  CONTAINS on a List does exact,
    /// case-insensitive element matching rather than substring matching.
    List(Vec<QValue>),
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
            QValue::List(_) => 5,
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
            QValue::List(items) => items
                .iter()
                .map(|v| v.as_sort_key())
                .collect::<Vec<_>>()
                .join(", "),
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
            QValue::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                write!(f, "[{}]", parts.join(", "))
            }
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
    /// Row-level computed columns (Formulas — #2921).
    /// Each formula has a name (output column) and an expression evaluated
    /// per row against that row's properties.
    pub formulas: Vec<Formula>,
}

/// A named computed-column formula (#2921).
#[derive(Debug, Clone)]
pub struct Formula {
    pub name: String,
    pub expr: FormulaExpr,
}

/// Expression AST for row-level computed columns (#2921).
///
/// Supports:
/// - Arithmetic: `col + col`, `col * 3`, `(col1 + col2) / 2`
/// - String: `concat(a, sep, b)`, `upper(s)`, `lower(s)`
/// - Conditional: `if(cond, then, else)`
/// - Date diff: `datediff(end, start)`, `dateadd(date, days)`
/// - Column references resolve to the row's property value
#[derive(Debug, Clone, PartialEq)]
pub enum FormulaExpr {
    /// Numeric literal: `42`, `3.14`.
    Number(f64),
    /// String literal: `"hello"`.
    Text(String),
    /// Date literal: `2026-07-16`.
    Date(NaiveDate),
    /// Column reference (bareword): `priority`, `status`, `due`.
    Column(String),
    /// Unary minus: `-expr`.
    Neg(Box<FormulaExpr>),
    /// Arithmetic: `lhs op rhs`.
    Add(Box<FormulaExpr>, Box<FormulaExpr>),
    Sub(Box<FormulaExpr>, Box<FormulaExpr>),
    Mul(Box<FormulaExpr>, Box<FormulaExpr>),
    Div(Box<FormulaExpr>, Box<FormulaExpr>),
    /// String concatenation: `concat(s1, sep, s2)`.
    Concat {
        left: Box<FormulaExpr>,
        sep: Box<FormulaExpr>,
        right: Box<FormulaExpr>,
    },
    /// Uppercase: `upper(s)`.
    Upper(Box<FormulaExpr>),
    /// Lowercase: `lower(s)`.
    Lower(Box<FormulaExpr>),
    /// Conditional: `if(cond, then, else)`.
    If {
        cond: Box<FormulaExpr>,
        then_branch: Box<FormulaExpr>,
        else_branch: Box<FormulaExpr>,
    },
    /// Date difference in days: `datediff(end, start)`.
    DateDiff {
        end: Box<FormulaExpr>,
        start: Box<FormulaExpr>,
    },
    /// Date addition: `dateadd(date, days)`.
    DateAdd {
        date: Box<FormulaExpr>,
        days: Box<FormulaExpr>,
    },
}

impl Default for Query {
    fn default() -> Self {
        Query {
            select: None,
            filter: Condition::True,
            order_by: None,
            limit: None,
            formulas: Vec::new(),
        }
    }
}

// ── Formula Parser ─────────────────────────────────────────────────────────

/// Tokens for the formula mini-language (#2921).
#[derive(Debug, Clone, PartialEq)]
enum FTok {
    Id(String),
    Num(f64),
    Str(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Comma,
}

fn is_f_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}
fn is_f_ident_cont(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

fn ftokenize(src: &str) -> Result<Vec<FTok>> {
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
            toks.push(FTok::Str(s));
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
            toks.push(FTok::Num(num));
            continue;
        }
        if is_f_ident_start(c) {
            let mut s = String::new();
            while let Some(&n) = chars.peek() {
                if is_f_ident_cont(n) {
                    s.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            // Try ISO date: 2026-07-16
            if let Ok(d) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
                // Only if it looks like a date (4 digits - 2 digits - 2 digits)
                if s.len() == 10 && s.chars().filter(|c| *c == '-').count() == 2 {
                    toks.push(FTok::Str(s));
                    continue;
                }
                let _ = d; // ignore
            }
            toks.push(FTok::Id(s));
            continue;
        }
        match c {
            '+' => {
                toks.push(FTok::Plus);
                chars.next();
            }
            '-' => {
                toks.push(FTok::Minus);
                chars.next();
            }
            '*' => {
                toks.push(FTok::Star);
                chars.next();
            }
            '/' => {
                toks.push(FTok::Slash);
                chars.next();
            }
            '(' => {
                toks.push(FTok::LParen);
                chars.next();
            }
            ')' => {
                toks.push(FTok::RParen);
                chars.next();
            }
            ',' => {
                toks.push(FTok::Comma);
                chars.next();
            }
            other => return Err(anyhow!("unexpected character in formula: {other}")),
        }
    }
    Ok(toks)
}

struct FPars {
    toks: Vec<FTok>,
    pos: usize,
}

impl FPars {
    fn new(toks: Vec<FTok>) -> Self {
        FPars { toks, pos: 0 }
    }

    fn peek(&self) -> Option<&FTok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<FTok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect_rparen(&mut self) -> Result<()> {
        match self.next() {
            Some(FTok::RParen) => Ok(()),
            other => Err(anyhow!("expected ), found {other:?}")),
        }
    }

    fn expect_comma(&mut self) -> Result<()> {
        match self.next() {
            Some(FTok::Comma) => Ok(()),
            other => Err(anyhow!("expected comma, found {other:?}")),
        }
    }

    /// Parse a complete formula expression string into a [`FormulaExpr`].
    fn parse_expr(&mut self) -> Result<FormulaExpr> {
        let expr = self.parse_addsub()?;
        if self.pos != self.toks.len() {
            return Err(anyhow!(
                "unexpected trailing tokens in formula at position {}",
                self.pos
            ));
        }
        Ok(expr)
    }

    /// addsub ::= muldiv (('+' | '-') muldiv)*
    fn parse_addsub(&mut self) -> Result<FormulaExpr> {
        let mut left = self.parse_muldiv()?;
        loop {
            match self.peek() {
                Some(FTok::Plus) => {
                    self.next();
                    let right = self.parse_muldiv()?;
                    left = FormulaExpr::Add(Box::new(left), Box::new(right));
                }
                Some(FTok::Minus) => {
                    self.next();
                    let right = self.parse_muldiv()?;
                    left = FormulaExpr::Sub(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    /// muldiv ::= unary (('*' | '/') unary)*
    fn parse_muldiv(&mut self) -> Result<FormulaExpr> {
        let mut left = self.parse_unary()?;
        loop {
            match self.peek() {
                Some(FTok::Star) => {
                    self.next();
                    let right = self.parse_unary()?;
                    left = FormulaExpr::Mul(Box::new(left), Box::new(right));
                }
                Some(FTok::Slash) => {
                    self.next();
                    let right = self.parse_unary()?;
                    left = FormulaExpr::Div(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    /// unary ::= `-` term | term
    fn parse_unary(&mut self) -> Result<FormulaExpr> {
        if matches!(self.peek(), Some(FTok::Minus)) {
            self.next();
            let expr = self.parse_unary()?;
            return Ok(FormulaExpr::Neg(Box::new(expr)));
        }
        self.parse_term()
    }

    /// term ::= NUMBER | STRING | ID | '(' expr ')' | ID '(' args... ')'
    fn parse_term(&mut self) -> Result<FormulaExpr> {
        match self.next() {
            Some(FTok::Num(n)) => Ok(FormulaExpr::Number(n)),
            Some(FTok::Str(s)) => {
                // Try to parse as ISO date if it looks like one.
                if let Ok(d) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
                    if s.len() == 10 && s.chars().filter(|c| *c == '-').count() == 2 {
                        return Ok(FormulaExpr::Date(d));
                    }
                }
                Ok(FormulaExpr::Text(s))
            }
            Some(FTok::Id(name)) => {
                // Peek ahead for '(' to distinguish column ref from function call.
                if matches!(self.peek(), Some(FTok::LParen)) {
                    self.next(); // consume '('
                    self.parse_function(name)
                } else {
                    Ok(FormulaExpr::Column(name))
                }
            }
            Some(FTok::LParen) => {
                let inner = self.parse_addsub()?;
                self.expect_rparen()?;
                Ok(inner)
            }
            other => Err(anyhow!("expected value, found {other:?}")),
        }
    }

    fn parse_function(&mut self, name: String) -> Result<FormulaExpr> {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "concat" => {
                let left = self.parse_addsub()?;
                self.expect_comma()?;
                let sep = self.parse_addsub()?;
                self.expect_comma()?;
                let right = self.parse_addsub()?;
                self.expect_rparen()?;
                Ok(FormulaExpr::Concat {
                    left: Box::new(left),
                    sep: Box::new(sep),
                    right: Box::new(right),
                })
            }
            "upper" => {
                let expr = self.parse_addsub()?;
                self.expect_rparen()?;
                Ok(FormulaExpr::Upper(Box::new(expr)))
            }
            "lower" => {
                let expr = self.parse_addsub()?;
                self.expect_rparen()?;
                Ok(FormulaExpr::Lower(Box::new(expr)))
            }
            "if" => {
                let cond = self.parse_addsub()?;
                self.expect_comma()?;
                let then_branch = self.parse_addsub()?;
                self.expect_comma()?;
                let else_branch = self.parse_addsub()?;
                self.expect_rparen()?;
                Ok(FormulaExpr::If {
                    cond: Box::new(cond),
                    then_branch: Box::new(then_branch),
                    else_branch: Box::new(else_branch),
                })
            }
            "datediff" => {
                let end = self.parse_addsub()?;
                self.expect_comma()?;
                let start = self.parse_addsub()?;
                self.expect_rparen()?;
                Ok(FormulaExpr::DateDiff {
                    end: Box::new(end),
                    start: Box::new(start),
                })
            }
            "dateadd" => {
                let date = self.parse_addsub()?;
                self.expect_comma()?;
                let days = self.parse_addsub()?;
                self.expect_rparen()?;
                Ok(FormulaExpr::DateAdd {
                    date: Box::new(date),
                    days: Box::new(days),
                })
            }
            _ => Err(anyhow!(
                "unknown function: {name}. Supported: concat, upper, lower, if, datediff, dateadd"
            )),
        }
    }
}

/// Parse a formula expression string into a [`FormulaExpr`].
pub fn parse_formula_expr(src: &str) -> Result<FormulaExpr> {
    let toks = ftokenize(src)?;
    if toks.is_empty() {
        return Err(anyhow!("empty formula expression"));
    }
    let mut parser = FPars::new(toks);
    parser.parse_expr()
}

/// Parse a `--formula name=expr` spec string into a [`Formula`].
pub fn parse_formula_spec(spec: &str) -> Result<Formula> {
    let eq = spec
        .find('=')
        .ok_or_else(|| anyhow!("invalid formula spec (expected NAME=expr): {spec}"))?;
    let name = spec[..eq].trim().to_string();
    let expr_str = spec[eq + 1..].trim();
    if name.is_empty() {
        anyhow::bail!("empty formula name in spec: {spec}");
    }
    if expr_str.is_empty() {
        anyhow::bail!("empty formula expression in spec: {spec}");
    }
    let expr = parse_formula_expr(expr_str)?;
    Ok(Formula { name, expr })
}

// ── Formula Evaluation ──────────────────────────────────────────────────────

/// Evaluate a [`FormulaExpr`] against a row's properties, producing a [`QValue`].
///
/// Column references resolve to the row's `props` map (or `Null` if absent).
/// The result is a typed [`QValue`] suitable for display, sorting, CSV export,
/// and downstream aggregations.
fn eval_formula(expr: &FormulaExpr, props: &HashMap<String, QValue>) -> QValue {
    match expr {
        FormulaExpr::Number(n) => QValue::Number(*n),
        FormulaExpr::Text(s) => QValue::Text(s.clone()),
        FormulaExpr::Date(d) => QValue::Date(*d),
        FormulaExpr::Column(name) => props.get(name).cloned().unwrap_or(QValue::Null),
        FormulaExpr::Neg(inner) => match eval_formula(inner, props) {
            QValue::Number(n) => QValue::Number(-n),
            _ => QValue::Null,
        },
        FormulaExpr::Add(l, r) => {
            let lv = eval_formula(l, props);
            let rv = eval_formula(r, props);
            match (lv, rv) {
                (QValue::Number(a), QValue::Number(b)) => QValue::Number(a + b),
                (QValue::Text(a), QValue::Text(b)) => QValue::Text(format!("{a}{b}")),
                (QValue::Number(a), QValue::Text(b)) => QValue::Text(format!("{a}{b}")),
                (QValue::Text(a), QValue::Number(b)) => QValue::Text(format!("{a}{b}")),
                _ => QValue::Null,
            }
        }
        FormulaExpr::Sub(l, r) => {
            let lv = eval_formula(l, props);
            let rv = eval_formula(r, props);
            match (lv, rv) {
                (QValue::Number(a), QValue::Number(b)) => QValue::Number(a - b),
                // Date difference: `due - start` → days.
                (QValue::Date(a), QValue::Date(b)) => QValue::Number((a - b).num_days() as f64),
                _ => QValue::Null,
            }
        }
        FormulaExpr::Mul(l, r) => {
            let lv = to_f64(&eval_formula(l, props));
            let rv = to_f64(&eval_formula(r, props));
            match (lv, rv) {
                (Some(a), Some(b)) => QValue::Number(a * b),
                _ => QValue::Null,
            }
        }
        FormulaExpr::Div(l, r) => {
            let lv = to_f64(&eval_formula(l, props));
            let rv = to_f64(&eval_formula(r, props));
            match (lv, rv) {
                (Some(a), Some(b)) if b != 0.0 => QValue::Number(a / b),
                _ => QValue::Null,
            }
        }
        FormulaExpr::Concat { left, sep, right } => {
            let ls = eval_formula(left, props).to_string();
            let ss = eval_formula(sep, props).to_string();
            let rs = eval_formula(right, props).to_string();
            QValue::Text(format!("{ls}{ss}{rs}"))
        }
        FormulaExpr::Upper(inner) => {
            let s = eval_formula(inner, props).to_string();
            QValue::Text(s.to_uppercase())
        }
        FormulaExpr::Lower(inner) => {
            let s = eval_formula(inner, props).to_string();
            QValue::Text(s.to_lowercase())
        }
        FormulaExpr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            let cv = eval_formula(cond, props);
            let truthy = match &cv {
                QValue::Bool(true) => true,
                QValue::Number(n) if *n != 0.0 => true,
                QValue::Text(s) if !s.is_empty() => true,
                _ => false,
            };
            if truthy {
                eval_formula(then_branch, props)
            } else {
                eval_formula(else_branch, props)
            }
        }
        FormulaExpr::DateDiff { end, start } => {
            let ev = to_date(&eval_formula(end, props));
            let sv = to_date(&eval_formula(start, props));
            match (ev, sv) {
                (Some(e), Some(s)) => QValue::Number((e - s).num_days() as f64),
                _ => QValue::Null,
            }
        }
        FormulaExpr::DateAdd { date, days } => {
            let dv = to_date(&eval_formula(date, props));
            let nd = to_f64(&eval_formula(days, props));
            match (dv, nd) {
                (Some(d), Some(n)) => {
                    let delta =
                        chrono::Duration::try_days(n as i64).unwrap_or(chrono::Duration::zero());
                    match d.checked_add_signed(delta) {
                        Some(new_date) => QValue::Date(new_date),
                        None => QValue::Null,
                    }
                }
                _ => QValue::Null,
            }
        }
    }
}

/// Apply all formulas to a single row, returning a map of formula_name → QValue.
fn evaluate_formulas(
    formulas: &[Formula],
    props: &HashMap<String, QValue>,
) -> HashMap<String, QValue> {
    let mut results = HashMap::new();
    for f in formulas {
        results.insert(f.name.clone(), eval_formula(&f.expr, props));
    }
    results
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
        // Look ahead from the CURRENT cursor (peekable iterator), never via
        // `src.find(c)` — that would jump to the first occurrence of `c`
        // anywhere in the source and wrongly read a two-char window from the
        // wrong position, splitting valid two-char operators (<= >= != <>) into
        // two single-char tokens whenever the leading char appears earlier.
        // `chars.clone().nth(1)` peeks the char AFTER the current cursor without
        // consuming anything from `chars` (index 0 is the current char `c`).
        let next = chars.clone().nth(1);
        let two = match (c, next) {
            ('<', Some('=')) | ('>', Some('=')) | ('!', Some('=')) | ('<', Some('>')) => {
                format!("{c}{}", next.unwrap())
            }
            _ => String::new(),
        };
        if !two.is_empty() {
            toks.push(Tok::Op(two));
            chars.next();
            chars.next();
        } else {
            match c {
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
            }
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
            formulas: Vec::new(),
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
        // List equality: same length, pairwise equal.
        (QValue::List(x), QValue::List(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| coerce_eq(a, b))
        }
        // Scalar vs List equality: check if any element equals the scalar.
        (scalar, QValue::List(items)) | (QValue::List(items), scalar)
            if !matches!(scalar, QValue::List(_)) =>
        {
            items.iter().any(|item| coerce_eq(item, scalar))
        }
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
        (QValue::List(x), QValue::List(y)) => {
            let xk: Vec<String> = x.iter().map(|v| v.as_sort_key()).collect();
            let yk: Vec<String> = y.iter().map(|v| v.as_sort_key()).collect();
            xk.cmp(&yk)
        }
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
                    // CONTAINS on a List: exact, case-insensitive element match
                    // (not substring). Fixes #2800.
                    (QValue::List(items), QValue::Text(r)) => {
                        let r_lower = r.to_lowercase();
                        items.iter().any(|item| match item {
                            QValue::Text(t) => t.to_lowercase() == r_lower,
                            other => other.to_string().to_lowercase() == r_lower,
                        })
                    }
                    // Text CONTAINS List — check if text matches any list element.
                    (QValue::Text(l), QValue::List(items)) => {
                        let l_lower = l.to_lowercase();
                        items.iter().any(|item| match item {
                            QValue::Text(t) => t.to_lowercase() == l_lower,
                            other => other.to_string().to_lowercase() == l_lower,
                        })
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
///
/// When `query.formulas` is non-empty, computed formula columns are appended
/// to every result row (#2921). Formulas are evaluated against the row's
/// **original** properties (before projection), so formulas work even when
/// the source column is omitted from SELECT.
pub fn query_records(records: &[Record], query: &Query) -> Vec<HashMap<String, QValue>> {
    let mut matched: Vec<&Record> = records
        .iter()
        .filter(|r| eval_cond(&query.filter, r))
        .collect();

    // When formulas reference columns, we sort after formula evaluation so
    // formulas that depend on order-sensitive data are correctly placed.
    // Pre-evaluate formula-only sorting key: if ORDER BY targets a formula
    // column, evaluate formulas *before* sorting.
    let order_field_is_formula = query
        .order_by
        .as_ref()
        .map(|(f, _)| query.formulas.iter().any(|fm| fm.name == *f))
        .unwrap_or(false);

    // Pre-compute formula values if ORDER BY references a formula column.
    let mut pre_formula_values: Vec<HashMap<String, QValue>> = Vec::new();
    if order_field_is_formula && !query.formulas.is_empty() {
        pre_formula_values = matched
            .iter()
            .map(|r| evaluate_formulas(&query.formulas, &r.props))
            .collect();
    }

    if let Some((field, dir)) = &query.order_by {
        if order_field_is_formula {
            // Sort using pre-computed formula values.
            let fvals = &pre_formula_values;
            let mut indexed: Vec<(usize, &Record)> =
                matched.iter().enumerate().map(|(i, r)| (i, *r)).collect();
            indexed.sort_by(|(ia, _a), (ib, _b)| {
                let va = fvals[*ia].get(field).cloned().unwrap_or(QValue::Null);
                let vb = fvals[*ib].get(field).cloned().unwrap_or(QValue::Null);
                let c = cmp_ord(&va, &vb);
                match dir {
                    Direction::Asc => c,
                    Direction::Desc => c.reverse(),
                }
            });
            matched = indexed.into_iter().map(|(_, r)| r).collect();
        } else {
            matched.sort_by(|a, b| {
                let c = cmp_ord(&get_prop(a, field), &get_prop(b, field));
                match dir {
                    Direction::Asc => c,
                    Direction::Desc => c.reverse(),
                }
            });
        }
    }

    if let Some(limit) = query.limit {
        matched.truncate(limit);
    }

    let has_formulas = !query.formulas.is_empty();
    matched
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
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
            // Append formula columns (#2921).
            if has_formulas {
                let formula_vals = if order_field_is_formula {
                    pre_formula_values[i].clone()
                } else {
                    evaluate_formulas(&query.formulas, &r.props)
                };
                for (name, value) in formula_vals {
                    row.insert(name, value);
                }
            }
            row
        })
        .collect()
}

// ── Output formatters (#2813) ────────────────────────────────────────────────

/// Collect a deterministic, stable column ordering across all rows (#2913).
///
/// Previously the CSV/Markdown formatters pulled columns from `rows[0].keys()`,
/// which iterates a `HashMap` in a **non-deterministic** order — so the same
/// `SELECT *` query could emit differently-ordered output between runs, breaking
/// downstream CSV consumers (pandas, Excel imports, etc.). This helper gathers
/// the union of keys from *every* row, then sorts them so output is stable:
///
/// - The synthetic `$path` column is always emitted first.
/// - All other columns follow in lexicographic (alphabetical) order.
///
/// Callers that need to preserve an explicit field order (e.g. a hand-written
/// `SELECT col1, col2`) should pass their own `columns` slice — as the CLI's
/// `format_as_csv` does — rather than relying on this union-based ordering.
fn collect_ordered_columns(rows: &[HashMap<String, QValue>]) -> Vec<String> {
    let mut keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for row in rows {
        for k in row.keys() {
            keys.insert(k.clone());
        }
    }
    let mut columns: Vec<String> = keys.into_iter().collect();
    // Move the synthetic `$path` column to the front for readability and so it
    // never shifts position when other properties are added or removed.
    if let Some(pos) = columns.iter().position(|c| c == "$path") {
        let path = columns.remove(pos);
        columns.insert(0, path);
    }
    columns
}

/// Format query result rows as CSV with header row.
/// Values containing commas, quotes, or newlines are properly escaped.
///
/// Column order is deterministic (`$path` first, then alphabetical) regardless
/// of `HashMap` iteration order (#2913).
pub fn format_rows_csv(rows: &[HashMap<String, QValue>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    // Deterministic column order: `$path` first, then alphabetical (#2913).
    let columns = collect_ordered_columns(rows);
    let header: Vec<String> = columns.iter().map(|c| csv_escape(c)).collect();
    let mut out = header.join(",");
    out.push('\n');
    for row in rows {
        let line: Vec<String> = columns
            .iter()
            .map(|c| {
                let v = row.get(c).cloned().unwrap_or(QValue::Null);
                csv_escape(&v.to_string())
            })
            .collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Format query result rows as a Markdown table (GFM-compatible).
///
/// Column order is deterministic (`$path` first, then alphabetical) regardless
/// of `HashMap` iteration order (#2913).
pub fn format_rows_md_table(rows: &[HashMap<String, QValue>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    // Deterministic column order: `$path` first, then alphabetical (#2913).
    let columns = collect_ordered_columns(rows);
    let header: Vec<String> = columns.iter().map(|c| c.to_string()).collect();
    let separator: Vec<String> = columns.iter().map(|_| "---".to_string()).collect();
    let mut out = format!(
        "| {} |\n| {} |\n",
        header.join(" | "),
        separator.join(" | ")
    );
    for row in rows {
        let cells: Vec<String> = columns
            .iter()
            .map(|c| {
                let v = row.get(c).cloned().unwrap_or(QValue::Null);
                md_escape(&v.to_string())
            })
            .collect();
        out.push_str(&format!("| {} |\n", cells.join(" | ")));
    }
    out
}

fn md_escape(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

/// Format query result rows as JSON (array of objects).
///
/// Object key order is deterministic: `serde_json::Map` is backed by a
/// `BTreeMap` (the `preserve_order` feature is not enabled), so keys are
/// serialized in sorted order regardless of `HashMap` iteration order (#2913).
pub fn format_rows_json(rows: &[HashMap<String, QValue>]) -> serde_json::Value {
    let arr: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let obj: serde_json::Map<String, serde_json::Value> = row
                .iter()
                .map(|(k, v)| {
                    let jv = match v {
                        QValue::Null => serde_json::Value::Null,
                        QValue::Bool(b) => serde_json::Value::Bool(*b),
                        QValue::Number(n) => serde_json::json!(*n),
                        QValue::Date(d) => serde_json::Value::String(d.to_string()),
                        QValue::Text(t) => serde_json::Value::String(t.clone()),
                        QValue::List(items) => {
                            let arr: Vec<serde_json::Value> = items
                                .iter()
                                .map(|i| match i {
                                    QValue::Null => serde_json::Value::Null,
                                    QValue::Bool(b) => serde_json::Value::Bool(*b),
                                    QValue::Number(n) => serde_json::json!(*n),
                                    QValue::Date(d) => serde_json::Value::String(d.to_string()),
                                    QValue::Text(t) => serde_json::Value::String(t.clone()),
                                    QValue::List(_) => serde_json::Value::String(i.to_string()),
                                })
                                .collect();
                            serde_json::Value::Array(arr)
                        }
                    };
                    (k.clone(), jv)
                })
                .collect();
            serde_json::Value::Object(obj)
        })
        .collect();
    serde_json::Value::Array(arr)
}

/// Load all vault notes as `Record`s for querying (#2813).
///
/// Reads every note file, extracts frontmatter YAML, and converts to a
/// [`Record`] via [`record_from_yaml`].  Notes without frontmatter produce
/// Records with just `$path`.
pub fn build_records_from_vault(
    context: &crate::storage::StorageContext,
) -> anyhow::Result<Vec<Record>> {
    use crate::models::SearchQuery;
    use crate::storage;
    let result = storage::search_notes_with_context(
        context,
        SearchQuery {
            text: String::new(),
            tags: Vec::new(),
            keywords: Vec::new(),
            limit: None, // load ALL notes for querying
            ..Default::default()
        },
    )?;
    let mut records = Vec::with_capacity(result.notes.len());
    for meta in &result.notes {
        let doc = storage::load_note_body_from_meta(meta)?;
        let (frontmatter_yaml, _body) = storage::notes::split_frontmatter_yaml(&doc.body)?;
        let rec = record_from_yaml(&meta.path, &frontmatter_yaml);
        records.push(rec);
    }
    Ok(records)
}

// ── Column aggregation / summarization (#2909) ──────────────────────────────

/// Aggregation function type for column summarization (#2909).
///
/// Mirrors the Obsidian Bases "Summaries" feature: each column in a table view
/// can display one or more aggregate statistics computed from the column values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AggFunction {
    /// Number of non-null values.
    Count,
    /// Sum of numeric values.
    Sum,
    /// Arithmetic mean of numeric values.
    Avg,
    /// Minimum value (works on all ordered types).
    Min,
    /// Maximum value (works on all ordered types).
    Max,
    /// Count of distinct non-null values.
    Unique,
    /// Number of null / absent values.
    Empty,
    /// Number of non-null values (alias for Count).
    Filled,
    /// Number of boolean `true` values.
    Checked,
    /// Number of boolean `false` values.
    Unchecked,
    /// Earliest date.
    Earliest,
    /// Latest date.
    Latest,
    /// Numeric range (Max - Min) or date-range string.
    Range,
}

impl std::fmt::Display for AggFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            AggFunction::Count => "Count",
            AggFunction::Sum => "Sum",
            AggFunction::Avg => "Avg",
            AggFunction::Min => "Min",
            AggFunction::Max => "Max",
            AggFunction::Unique => "Unique",
            AggFunction::Empty => "Empty",
            AggFunction::Filled => "Filled",
            AggFunction::Checked => "Checked",
            AggFunction::Unchecked => "Unchecked",
            AggFunction::Earliest => "Earliest",
            AggFunction::Latest => "Latest",
            AggFunction::Range => "Range",
        };
        write!(f, "{name}")
    }
}

/// Parse an `AggFunction` from its display name (case-insensitive).
pub fn agg_function_from_str(s: &str) -> Option<AggFunction> {
    match s.to_ascii_lowercase().as_str() {
        "count" => Some(AggFunction::Count),
        "sum" => Some(AggFunction::Sum),
        "avg" | "average" => Some(AggFunction::Avg),
        "min" | "minimum" => Some(AggFunction::Min),
        "max" | "maximum" => Some(AggFunction::Max),
        "unique" | "distinct" => Some(AggFunction::Unique),
        "empty" | "null" => Some(AggFunction::Empty),
        "filled" | "nonnull" | "notnull" => Some(AggFunction::Filled),
        "checked" | "true" => Some(AggFunction::Checked),
        "unchecked" | "false" => Some(AggFunction::Unchecked),
        "earliest" | "min_date" => Some(AggFunction::Earliest),
        "latest" | "max_date" => Some(AggFunction::Latest),
        "range" => Some(AggFunction::Range),
        _ => None,
    }
}

/// Compute a single aggregation function over a column (slice of values).
///
/// This is the core computational primitive. It works on any value type and
/// returns the most appropriate result type for each function.  When the input
/// is empty or no values satisfy the function's type requirement, returns
/// [`QValue::Null`].
pub fn summarize_column(values: &[QValue], func: AggFunction) -> QValue {
    let non_null: Vec<&QValue> = values
        .iter()
        .filter(|v| !matches!(v, QValue::Null))
        .collect();
    let n = non_null.len();
    let total = values.len();

    match func {
        AggFunction::Empty => QValue::Number((total - n) as f64),
        AggFunction::Filled | AggFunction::Count => QValue::Number(n as f64),

        AggFunction::Unique => {
            let mut seen: Vec<String> = Vec::new();
            for v in non_null {
                let key = v.to_string();
                if !seen.contains(&key) {
                    seen.push(key);
                }
            }
            QValue::Number(seen.len() as f64)
        }

        AggFunction::Sum => {
            let nums: Vec<f64> = non_null.iter().filter_map(|v| to_f64(v)).collect();
            if nums.is_empty() {
                QValue::Null
            } else {
                QValue::Number(nums.iter().sum())
            }
        }

        AggFunction::Avg => {
            let nums: Vec<f64> = non_null.iter().filter_map(|v| to_f64(v)).collect();
            if nums.is_empty() {
                QValue::Null
            } else {
                QValue::Number(nums.iter().sum::<f64>() / nums.len() as f64)
            }
        }

        AggFunction::Min => non_null
            .into_iter()
            .cloned()
            .min_by(cmp_qvalue)
            .unwrap_or(QValue::Null),

        AggFunction::Max => non_null
            .into_iter()
            .cloned()
            .max_by(cmp_qvalue)
            .unwrap_or(QValue::Null),

        AggFunction::Range => {
            let nums: Vec<f64> = non_null.iter().filter_map(|v| to_f64(v)).collect();
            if nums.len() >= 2 {
                let min = nums.iter().cloned().fold(f64::INFINITY, f64::min);
                let max = nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                QValue::Number(max - min)
            } else {
                // Try date range
                let dates: Vec<NaiveDate> = non_null.iter().filter_map(|v| to_date(v)).collect();
                if dates.len() >= 2 {
                    let min = *dates.iter().min().unwrap();
                    let max = *dates.iter().max().unwrap();
                    let days = (max - min).num_days();
                    QValue::Text(format!("{days} days"))
                } else {
                    QValue::Null
                }
            }
        }

        AggFunction::Checked => {
            let count = non_null
                .iter()
                .filter(|v| matches!(v, QValue::Bool(true)))
                .count();
            QValue::Number(count as f64)
        }

        AggFunction::Unchecked => {
            let count = non_null
                .iter()
                .filter(|v| matches!(v, QValue::Bool(false)))
                .count();
            QValue::Number(count as f64)
        }

        AggFunction::Earliest => {
            let dates: Vec<NaiveDate> = non_null.iter().filter_map(|v| to_date(v)).collect();
            dates
                .into_iter()
                .min()
                .map(QValue::Date)
                .unwrap_or(QValue::Null)
        }

        AggFunction::Latest => {
            let dates: Vec<NaiveDate> = non_null.iter().filter_map(|v| to_date(v)).collect();
            dates
                .into_iter()
                .max()
                .map(QValue::Date)
                .unwrap_or(QValue::Null)
        }
    }
}

/// Compute multiple aggregation functions over multiple columns.
///
/// Returns a map from column name to a vector of `(function, result)` pairs
/// in the same order as `specs` functions.
pub fn summarize_records(
    records: &[HashMap<String, QValue>],
    column_specs: &[(&str, Vec<AggFunction>)],
) -> HashMap<String, Vec<(AggFunction, QValue)>> {
    let mut results = HashMap::new();
    for (col, funcs) in column_specs {
        let values: Vec<QValue> = records
            .iter()
            .map(|row| row.get(*col).cloned().unwrap_or(QValue::Null))
            .collect();
        let agg: Vec<(AggFunction, QValue)> = funcs
            .iter()
            .map(|f| (*f, summarize_column(&values, *f)))
            .collect();
        results.insert(col.to_string(), agg);
    }
    results
}

/// Format summarization results for human-readable display.
///
/// Produces a compact table where each column's aggregations are shown
/// as `col: Func1=val1, Func2=val2, ...`.
pub fn format_summaries(summaries: &HashMap<String, Vec<(AggFunction, QValue)>>) -> String {
    if summaries.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::new();
    // Sort columns for stable output
    let mut columns: Vec<&String> = summaries.keys().collect();
    columns.sort();
    for col in columns {
        if let Some(funcs) = summaries.get(col) {
            let parts: Vec<String> = funcs.iter().map(|(f, v)| format!("{f}={v}")).collect();
            lines.push(format!("{col}: {}", parts.join(", ")));
        }
    }
    format!("📊 Column Summaries\n{}\n", lines.join("\n"))
}

// ── Internal helpers ────────────────────────────────────────────────────────

/// Extract a numeric `f64` from a QValue (Numbers and numeric-looking Text).
fn to_f64(v: &QValue) -> Option<f64> {
    match v {
        QValue::Number(n) => Some(*n),
        QValue::Text(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Extract a `NaiveDate` from a QValue.
fn to_date(v: &QValue) -> Option<NaiveDate> {
    match v {
        QValue::Date(d) => Some(*d),
        QValue::Text(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").ok(),
        _ => None,
    }
}

/// Compare two `QValue`s for ordering (used by min/max).
fn cmp_qvalue(a: &QValue, b: &QValue) -> std::cmp::Ordering {
    cmp_ord(a, b)
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
        // YAML sequences are now stored as QValue::List for proper CONTAINS
        // semantics (exact element match). Fixes #2800.
        Yaml::Sequence(seq) => QValue::List(seq.iter().map(yaml_to_qvalue).collect()),
        // Mappings are still stored as Text (Bases relations can be refined
        // later without breaking the engine).
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

// ── GROUP BY / Bases view grouping (#3568) ───────────────────────────────────

/// A group produced by [`group_records_by`].
///
/// Each group represents one column in a Kanban view (or one bucket in any
/// other Bases-style view): the group key (e.g. `"Todo"`, `"Doing"`, `"Done"`)
/// and the records that fall into it.
#[derive(Debug, Clone)]
pub struct RecordGroup {
    /// Human-readable group key (e.g. `"active"`, `"done"`, or `"Unfiled"`
    /// for records missing the grouping property).
    pub key: String,
    /// The raw [`QValue`] used for grouping, enabling structured output.
    pub value: QValue,
    /// Number of records in this group.
    pub count: usize,
    /// The `$path` values of all records in this group, in input order.
    pub paths: Vec<String>,
}

/// Group records by the value of a specified property (#3568).
///
/// This is the backend foundation for Bases Kanban / Gallery views: given a
/// group-by field (e.g. `status`, `project`), records are partitioned into
/// groups.  Records with a null or absent value for the field go into a
/// special **"Unfiled"** group.
///
/// Group ordering is **deterministic**:
/// - Non-null groups appear in alphabetical order by display key.
/// - The "Unfiled" (null) group always appears **last**.
///
/// # Examples
///
/// ```
/// use vaultpilot_lib::vault_query::{Record, QValue, group_records_by};
///
/// let records = vec![
///     Record::new("a.md").with_prop("status", QValue::Text("active".into())),
///     Record::new("b.md").with_prop("status", QValue::Text("done".into())),
///     Record::new("c.md").with_prop("status", QValue::Text("active".into())),
///     Record::new("d.md"), // no status — goes to Unfiled
/// ];
/// let groups = group_records_by(&records, "status");
/// assert_eq!(groups.len(), 3);
/// assert_eq!(groups[0].key, "active");
/// assert_eq!(groups[0].count, 2);
/// assert_eq!(groups[1].key, "done");
/// assert_eq!(groups[2].key, "Unfiled");
/// ```
pub fn group_records_by(records: &[Record], field: &str) -> Vec<RecordGroup> {
    use std::collections::BTreeMap;

    // Partition into (key, value, paths) keyed by the display string.
    // We use BTreeMap for automatic alphabetical ordering of string keys.
    let mut groups: BTreeMap<String, (QValue, Vec<String>)> = BTreeMap::new();
    let mut unfiled_paths: Vec<String> = Vec::new();

    for r in records {
        match r.props.get(field) {
            Some(QValue::Null) | None => {
                unfiled_paths.push(r.path.clone());
            }
            Some(val) => {
                let display = qvalue_display_key(val);
                groups
                    .entry(display)
                    .or_insert_with(|| (val.clone(), Vec::new()))
                    .1
                    .push(r.path.clone());
            }
        }
    }

    let mut result: Vec<RecordGroup> = groups
        .into_iter()
        .map(|(key, (value, paths))| RecordGroup {
            count: paths.len(),
            key,
            value,
            paths,
        })
        .collect();

    // Unfiled group always last.
    if !unfiled_paths.is_empty() {
        result.push(RecordGroup {
            key: "Unfiled".to_string(),
            value: QValue::Null,
            count: unfiled_paths.len(),
            paths: unfiled_paths,
        });
    }

    result
}

/// Produce a deterministic display key for a [`QValue`] used in grouping.
///
/// - `Text` → the string itself
/// - `Number` → the number formatted (without trailing zeros)
/// - `Bool` → `"true"` / `"false"`
/// - `Date` → ISO date string
/// - `List` → `"multi"` (each element is a sub-key; not expanded here)
fn qvalue_display_key(val: &QValue) -> String {
    match val {
        QValue::Null => "Unfiled".to_string(),
        QValue::Text(t) => t.clone(),
        QValue::Number(n) => {
            // Format without trailing `.0` for whole numbers.
            if n.fract() == 0.0 {
                format!("{}", *n as i64)
            } else {
                format!("{n}")
            }
        }
        QValue::Bool(b) => b.to_string(),
        QValue::Date(d) => d.to_string(),
        QValue::List(_) => "multi".to_string(),
    }
}

/// Format grouped records as JSON (for front-end Kanban/Gallery views).
///
/// Output shape:
///
/// ```json
/// [
///   {"key": "active", "value": "active", "count": 2, "paths": ["a.md", "c.md"]},
///   {"key": "done",   "value": "done",   "count": 1, "paths": ["b.md"]},
///   {"key": "Unfiled", "value": null,    "count": 1, "paths": ["d.md"]}
/// ]
/// ```
pub fn format_groups_json(groups: &[RecordGroup]) -> serde_json::Value {
    let arr: Vec<serde_json::Value> = groups
        .iter()
        .map(|g| {
            let value_json = match &g.value {
                QValue::Null => serde_json::Value::Null,
                QValue::Bool(b) => serde_json::Value::Bool(*b),
                QValue::Number(n) => serde_json::json!(*n),
                QValue::Date(d) => serde_json::Value::String(d.to_string()),
                QValue::Text(t) => serde_json::Value::String(t.clone()),
                QValue::List(items) => {
                    let arr: Vec<serde_json::Value> = items
                        .iter()
                        .map(|i| match i {
                            QValue::Null => serde_json::Value::Null,
                            QValue::Bool(b) => serde_json::Value::Bool(*b),
                            QValue::Number(n) => serde_json::json!(*n),
                            QValue::Date(d) => serde_json::Value::String(d.to_string()),
                            QValue::Text(t) => serde_json::Value::String(t.clone()),
                            QValue::List(_) => serde_json::Value::String(i.to_string()),
                        })
                        .collect();
                    serde_json::Value::Array(arr)
                }
            };
            serde_json::json!({
                "key": g.key,
                "value": value_json,
                "count": g.count,
                "paths": g.paths,
            })
        })
        .collect();
    serde_json::Value::Array(arr)
}

/// Format grouped records as a human-readable Markdown table.
///
/// Useful for CLI output and quick inspection.
pub fn format_groups_md(groups: &[RecordGroup]) -> String {
    let mut out = String::from("| Group | Count | Notes |\n|-------|-------|-------|\n");
    for g in groups {
        let paths = g.paths.join(", ");
        out.push_str(&format!("| {} | {} | {} |\n", g.key, g.count, paths));
    }
    out
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
    fn contains_list_exact_match_regression_2800() {
        // Regression #2800: CONTAINS on a YAML list property must do exact
        // element matching, not substring matching.
        let records = vec![
            rec("note1.md").with_prop(
                "tags",
                QValue::List(vec![
                    QValue::Text("rust".into()),
                    QValue::Text("ai".into()),
                    QValue::Text("notes".into()),
                ]),
            ),
            rec("note2.md").with_prop(
                "tags",
                QValue::List(vec![
                    QValue::Text("mobile".into()),
                    QValue::Text("react".into()),
                ]),
            ),
        ];

        // Exact match: "rust" should match note1
        let q = parse_query(r#"SELECT * WHERE tags CONTAINS "rust""#).unwrap();
        let rows = query_records(&records, &q);
        assert_eq!(rows.len(), 1, "exact match 'rust' should hit note1 only");
        assert_eq!(rows[0].get("$path"), Some(&QValue::Text("note1.md".into())));

        // Substring non-match: "rus" must NOT match (only "rust" exists)
        let q = parse_query(r#"SELECT * WHERE tags CONTAINS "rus""#).unwrap();
        let rows = query_records(&records, &q);
        assert_eq!(rows.len(), 0, "substring 'rus' must not match 'rust'");

        // Substring non-match: "note" must NOT match (only "notes" exists)
        let q = parse_query(r#"SELECT * WHERE tags CONTAINS "note""#).unwrap();
        let rows = query_records(&records, &q);
        assert_eq!(rows.len(), 0, "substring 'note' must not match 'notes'");

        // Substring non-match: "a" must NOT match (only "ai" exists)
        let q = parse_query(r#"SELECT * WHERE tags CONTAINS "a""#).unwrap();
        let rows = query_records(&records, &q);
        assert_eq!(rows.len(), 0, "substring 'a' must not match 'ai'");

        // Case-insensitive exact match: "RUST" should match note1
        let q = parse_query(r#"SELECT * WHERE tags CONTAINS "RUST""#).unwrap();
        let rows = query_records(&records, &q);
        assert_eq!(rows.len(), 1, "case-insensitive 'RUST' should match 'rust'");
    }

    #[test]
    fn contains_list_via_yaml_frontmatter_2800() {
        // Integration test: YAML frontmatter list → QValue::List → CONTAINS
        let mut m = serde_yaml_ng::Mapping::new();
        m.insert(
            Yaml::String("tags".into()),
            Yaml::Sequence(vec![
                Yaml::String("rust".into()),
                Yaml::String("ai".into()),
                Yaml::String("notes".into()),
            ]),
        );
        let record = record_from_yaml("x.md", &m);
        assert_eq!(
            record.props.get("tags"),
            Some(&QValue::List(vec![
                QValue::Text("rust".into()),
                QValue::Text("ai".into()),
                QValue::Text("notes".into()),
            ]))
        );

        let q = parse_query(r#"SELECT * WHERE tags CONTAINS "rust""#).unwrap();
        let rows = query_records(&[record], &q);
        assert_eq!(rows.len(), 1);

        // Substring should NOT match
        let q = parse_query(r#"SELECT * WHERE tags CONTAINS "rus""#).unwrap();
        let rows = query_records(
            &[record_from_yaml("x.md", &{
                let mut m = serde_yaml_ng::Mapping::new();
                m.insert(
                    Yaml::String("tags".into()),
                    Yaml::Sequence(vec![
                        Yaml::String("rust".into()),
                        Yaml::String("ai".into()),
                        Yaml::String("notes".into()),
                    ]),
                );
                m
            })],
            &q,
        );
        assert_eq!(rows.len(), 0);
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

    #[test]
    fn two_char_operators_not_split_regression_2776() {
        // Regression #2776: a two-char operator (<= >= != <>) appearing after an
        // earlier single-char occurrence of its leading char must still be
        // tokenized as ONE Op token, not split into two single-char tokens.
        // The buggy tokenizer used `src.find(c)`, which jumped to the *first*
        // occurrence of the leading char anywhere in the source and read the
        // two-char window from the wrong position.
        let toks = tokenize("a < b AND c <= d").expect("tokenize failed");
        assert!(
            toks.iter().any(|t| matches!(t, Tok::Op(o) if o == "<=")),
            "expected a single `<=` Op token; `<=` was split"
        );
        assert!(
            !toks.iter().any(|t| matches!(t, Tok::Op(o) if o == "=")),
            "found a stray standalone `=` — `<=` was split into `<` and `=`"
        );

        let toks = tokenize("x > y AND z >= w").expect("tokenize failed");
        assert!(
            toks.iter().any(|t| matches!(t, Tok::Op(o) if o == ">=")),
            "expected a single `>=` Op token; `>=` was split"
        );

        let toks = tokenize("m < n OR o <> p").expect("tokenize failed");
        assert!(
            toks.iter().any(|t| matches!(t, Tok::Op(o) if o == "<>")),
            "expected a single `<>` Op token; `<>` was split"
        );

        // `!=` is preserved even though `=` appears earlier.
        let toks = tokenize("p = q AND r != s").expect("tokenize failed");
        assert!(
            toks.iter().any(|t| matches!(t, Tok::Op(o) if o == "!=")),
            "expected a single `!=` Op token"
        );

        // End-to-end: a query with both a single and a two-char operator parses
        // and filters correctly (a.md priority 3 and c.md priority 5 qualify).
        let q = parse_query(r#"SELECT * WHERE priority >= 3 AND status != "x""#).unwrap();
        assert_eq!(query_records(&sample_records(), &q).len(), 2);
    }

    // ── Aggregation / summarization tests (#2909) ─────────────────────────

    #[test]
    fn summarize_count_and_empty() {
        let values = vec![
            QValue::Number(1.0),
            QValue::Number(2.0),
            QValue::Null,
            QValue::Number(4.0),
            QValue::Null,
        ];
        assert_eq!(
            summarize_column(&values, AggFunction::Count),
            QValue::Number(3.0)
        );
        assert_eq!(
            summarize_column(&values, AggFunction::Empty),
            QValue::Number(2.0)
        );
        assert_eq!(
            summarize_column(&values, AggFunction::Filled),
            QValue::Number(3.0)
        );
    }

    #[test]
    fn summarize_sum_avg_min_max() {
        let values = vec![
            QValue::Number(10.0),
            QValue::Number(20.0),
            QValue::Number(30.0),
            QValue::Null,
        ];
        assert_eq!(
            summarize_column(&values, AggFunction::Sum),
            QValue::Number(60.0)
        );
        assert!(
            (summarize_column(&values, AggFunction::Avg)
                .to_string()
                .parse::<f64>()
                .unwrap()
                - 20.0)
                .abs()
                < 1e-10
        );
        assert_eq!(
            summarize_column(&values, AggFunction::Min),
            QValue::Number(10.0)
        );
        assert_eq!(
            summarize_column(&values, AggFunction::Max),
            QValue::Number(30.0)
        );
    }

    #[test]
    fn summarize_range_numeric() {
        let values = vec![
            QValue::Number(5.0),
            QValue::Number(15.0),
            QValue::Number(3.0),
        ];
        let r = summarize_column(&values, AggFunction::Range);
        assert_eq!(r, QValue::Number(12.0)); // 15 - 3
    }

    #[test]
    fn summarize_range_date() {
        let values = vec![
            QValue::Date(NaiveDate::from_ymd_opt(2026, 1, 10).unwrap()),
            QValue::Date(NaiveDate::from_ymd_opt(2026, 1, 15).unwrap()),
            QValue::Date(NaiveDate::from_ymd_opt(2026, 1, 8).unwrap()),
        ];
        let r = summarize_column(&values, AggFunction::Range);
        assert!(r.to_string().contains("7")); // 15 - 8 = 7 days
    }

    #[test]
    fn summarize_unique() {
        let values = vec![
            QValue::Text("a".into()),
            QValue::Text("b".into()),
            QValue::Text("a".into()),
            QValue::Null,
            QValue::Text("c".into()),
        ];
        assert_eq!(
            summarize_column(&values, AggFunction::Unique),
            QValue::Number(3.0)
        );
    }

    #[test]
    fn summarize_checked_unchecked() {
        let values = vec![
            QValue::Bool(true),
            QValue::Bool(false),
            QValue::Bool(true),
            QValue::Null,
            QValue::Bool(false),
        ];
        assert_eq!(
            summarize_column(&values, AggFunction::Checked),
            QValue::Number(2.0)
        );
        assert_eq!(
            summarize_column(&values, AggFunction::Unchecked),
            QValue::Number(2.0)
        );
    }

    #[test]
    fn summarize_earliest_latest() {
        let values = vec![
            QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 10).unwrap()),
            QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
            QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()),
        ];
        assert_eq!(
            summarize_column(&values, AggFunction::Earliest),
            QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap())
        );
        assert_eq!(
            summarize_column(&values, AggFunction::Latest),
            QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 20).unwrap())
        );
    }

    #[test]
    fn summarize_empty_input() {
        let values: Vec<QValue> = vec![];
        assert_eq!(summarize_column(&values, AggFunction::Sum), QValue::Null);
        assert_eq!(summarize_column(&values, AggFunction::Avg), QValue::Null);
        assert_eq!(summarize_column(&values, AggFunction::Min), QValue::Null);
        assert_eq!(
            summarize_column(&values, AggFunction::Count),
            QValue::Number(0.0)
        );
        assert_eq!(
            summarize_column(&values, AggFunction::Empty),
            QValue::Number(0.0)
        );
        assert_eq!(
            summarize_column(&values, AggFunction::Unique),
            QValue::Number(0.0)
        );
    }

    #[test]
    fn summarize_all_null() {
        let values = vec![QValue::Null, QValue::Null, QValue::Null];
        assert_eq!(summarize_column(&values, AggFunction::Sum), QValue::Null);
        assert_eq!(
            summarize_column(&values, AggFunction::Count),
            QValue::Number(0.0)
        );
        assert_eq!(
            summarize_column(&values, AggFunction::Empty),
            QValue::Number(3.0)
        );
        assert_eq!(summarize_column(&values, AggFunction::Min), QValue::Null);
    }

    #[test]
    fn summarize_records_multiple_columns() {
        let records = query_records(&sample_records(), &Query::default());
        let specs = vec![
            (
                "priority",
                vec![
                    AggFunction::Sum,
                    AggFunction::Avg,
                    AggFunction::Min,
                    AggFunction::Max,
                ],
            ),
            (
                "status",
                vec![AggFunction::Count, AggFunction::Unique, AggFunction::Empty],
            ),
        ];
        let summaries = summarize_records(&records, &specs);
        assert!(summaries.contains_key("priority"));
        assert!(summaries.contains_key("status"));

        let prio = summaries.get("priority").unwrap();
        // Sum of priorities: 3+1+5+null(d)=9
        assert_eq!(
            prio.iter().find(|(f, _)| *f == AggFunction::Sum).unwrap().1,
            QValue::Number(9.0)
        );
        // Avg: 9/3 = 3
        let avg_val = prio
            .iter()
            .find(|(f, _)| *f == AggFunction::Avg)
            .unwrap()
            .1
            .to_string()
            .parse::<f64>()
            .unwrap();
        assert!((avg_val - 3.0).abs() < 1e-10);
        // Min: 1
        assert_eq!(
            prio.iter().find(|(f, _)| *f == AggFunction::Min).unwrap().1,
            QValue::Number(1.0)
        );
        // Max: 5
        assert_eq!(
            prio.iter().find(|(f, _)| *f == AggFunction::Max).unwrap().1,
            QValue::Number(5.0)
        );

        let status = summaries.get("status").unwrap();
        // Count of non-null statuses: a,b,c,d = 4
        assert_eq!(
            status
                .iter()
                .find(|(f, _)| *f == AggFunction::Count)
                .unwrap()
                .1,
            QValue::Number(4.0)
        );
        // Unique: active, done, backlog = 3
        assert_eq!(
            status
                .iter()
                .find(|(f, _)| *f == AggFunction::Unique)
                .unwrap()
                .1,
            QValue::Number(3.0)
        );
        // Empty: 0
        assert_eq!(
            status
                .iter()
                .find(|(f, _)| *f == AggFunction::Empty)
                .unwrap()
                .1,
            QValue::Number(0.0)
        );
    }

    #[test]
    fn agg_function_from_str_parses_all() {
        assert_eq!(agg_function_from_str("count"), Some(AggFunction::Count));
        assert_eq!(agg_function_from_str("SUM"), Some(AggFunction::Sum));
        assert_eq!(agg_function_from_str("Average"), Some(AggFunction::Avg));
        assert_eq!(agg_function_from_str("min"), Some(AggFunction::Min));
        assert_eq!(agg_function_from_str("MAX"), Some(AggFunction::Max));
        assert_eq!(agg_function_from_str("unique"), Some(AggFunction::Unique));
        assert_eq!(agg_function_from_str("empty"), Some(AggFunction::Empty));
        assert_eq!(agg_function_from_str("Filled"), Some(AggFunction::Filled));
        assert_eq!(agg_function_from_str("checked"), Some(AggFunction::Checked));
        assert_eq!(
            agg_function_from_str("UNCHECKED"),
            Some(AggFunction::Unchecked)
        );
        assert_eq!(
            agg_function_from_str("Earliest"),
            Some(AggFunction::Earliest)
        );
        assert_eq!(agg_function_from_str("LATEST"), Some(AggFunction::Latest));
        assert_eq!(agg_function_from_str("range"), Some(AggFunction::Range));
        assert_eq!(agg_function_from_str("bogus"), None);
        assert_eq!(agg_function_from_str("distinct"), Some(AggFunction::Unique));
        assert_eq!(agg_function_from_str("null"), Some(AggFunction::Empty));
        assert_eq!(agg_function_from_str("notnull"), Some(AggFunction::Filled));
        assert_eq!(agg_function_from_str("minimum"), Some(AggFunction::Min));
        assert_eq!(agg_function_from_str("maximum"), Some(AggFunction::Max));
    }

    #[test]
    fn summarize_mixed_type_column() {
        // Mixed types: numbers should still work for numeric aggs
        let values = vec![
            QValue::Number(42.0),
            QValue::Text("hello".into()),
            QValue::Bool(true),
            QValue::Null,
        ];
        assert_eq!(
            summarize_column(&values, AggFunction::Count),
            QValue::Number(3.0)
        );
        assert_eq!(
            summarize_column(&values, AggFunction::Sum),
            QValue::Number(42.0)
        );
    }

    // ── GROUP BY / Bases view tests (#3568) ─────────────────────────────────

    #[test]
    fn group_by_status_basic() {
        let records = vec![
            rec("a.md").with_prop("status", QValue::Text("active".into())),
            rec("b.md").with_prop("status", QValue::Text("done".into())),
            rec("c.md").with_prop("status", QValue::Text("active".into())),
        ];
        let groups = group_records_by(&records, "status");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].key, "active");
        assert_eq!(groups[0].count, 2);
        assert_eq!(groups[1].key, "done");
        assert_eq!(groups[1].count, 1);
    }

    #[test]
    fn group_by_with_unfiled() {
        let records = vec![
            rec("a.md").with_prop("status", QValue::Text("active".into())),
            rec("b.md"),                                   // missing status
            rec("c.md").with_prop("status", QValue::Null), // explicit null
        ];
        let groups = group_records_by(&records, "status");
        assert_eq!(groups.len(), 2);
        // Unfiled is always last
        assert_eq!(groups[1].key, "Unfiled");
        assert_eq!(groups[1].count, 2);
        assert!(groups[1].paths.contains(&"b.md".to_string()));
        assert!(groups[1].paths.contains(&"c.md".to_string()));
    }

    #[test]
    fn group_by_alphabetical_order() {
        let records = vec![
            rec("a.md").with_prop("priority", QValue::Text("zenith".into())),
            rec("b.md").with_prop("priority", QValue::Text("alpha".into())),
            rec("c.md").with_prop("priority", QValue::Text("mid".into())),
        ];
        let groups = group_records_by(&records, "priority");
        assert_eq!(groups.len(), 3);
        // Alphabetical, not input order
        assert_eq!(groups[0].key, "alpha");
        assert_eq!(groups[1].key, "mid");
        assert_eq!(groups[2].key, "zenith");
    }

    #[test]
    fn group_by_number_field() {
        let records = vec![
            rec("a.md").with_prop("sprint", QValue::Number(1.0)),
            rec("b.md").with_prop("sprint", QValue::Number(2.0)),
            rec("c.md").with_prop("sprint", QValue::Number(1.0)),
        ];
        let groups = group_records_by(&records, "sprint");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].key, "1");
        assert_eq!(groups[0].count, 2);
        assert_eq!(groups[1].key, "2");
    }

    #[test]
    fn group_by_bool_field() {
        let records = vec![
            rec("a.md").with_prop("archived", QValue::Bool(true)),
            rec("b.md").with_prop("archived", QValue::Bool(false)),
            rec("c.md").with_prop("archived", QValue::Bool(true)),
        ];
        let groups = group_records_by(&records, "archived");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].key, "false");
        assert_eq!(groups[1].key, "true");
    }

    #[test]
    fn group_by_date_field() {
        let records = vec![
            rec("a.md").with_prop(
                "due",
                QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
            ),
            rec("b.md").with_prop(
                "due",
                QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 2).unwrap()),
            ),
            rec("c.md").with_prop(
                "due",
                QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
            ),
        ];
        let groups = group_records_by(&records, "due");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].key, "2026-07-01");
        assert_eq!(groups[0].count, 2);
        assert_eq!(groups[1].key, "2026-07-02");
    }

    #[test]
    fn group_by_empty_records() {
        let groups = group_records_by(&[], "status");
        assert!(groups.is_empty());
    }

    #[test]
    fn group_by_all_unfiled() {
        let records = vec![rec("a.md"), rec("b.md"), rec("c.md")];
        let groups = group_records_by(&records, "status");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].key, "Unfiled");
        assert_eq!(groups[0].count, 3);
    }

    #[test]
    fn group_by_preserves_paths_order() {
        let records = vec![
            rec("z.md").with_prop("status", QValue::Text("active".into())),
            rec("a.md").with_prop("status", QValue::Text("active".into())),
            rec("m.md").with_prop("status", QValue::Text("active".into())),
        ];
        let groups = group_records_by(&records, "status");
        assert_eq!(groups.len(), 1);
        // Paths in input order, not sorted
        assert_eq!(groups[0].paths, vec!["z.md", "a.md", "m.md"]);
    }

    #[test]
    fn group_by_list_value() {
        // List-valued properties go into a single "multi" group.
        let records = vec![
            rec("a.md").with_prop(
                "tags",
                QValue::List(vec![QValue::Text("rust".into()), QValue::Text("ai".into())]),
            ),
            rec("b.md").with_prop("tags", QValue::List(vec![QValue::Text("rust".into())])),
        ];
        let groups = group_records_by(&records, "tags");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].key, "multi");
        assert_eq!(groups[0].count, 2);
    }

    #[test]
    fn format_groups_json_structure() {
        let records = vec![
            rec("a.md").with_prop("status", QValue::Text("active".into())),
            rec("b.md").with_prop("status", QValue::Text("done".into())),
            rec("c.md"), // unfiled
        ];
        let groups = group_records_by(&records, "status");
        let json = format_groups_json(&groups);
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        // active group
        assert_eq!(arr[0]["key"], "active");
        assert_eq!(arr[0]["count"], 1);
        assert_eq!(arr[0]["value"], "active");
        // done group
        assert_eq!(arr[1]["key"], "done");
        // unfiled group
        assert_eq!(arr[2]["key"], "Unfiled");
        assert_eq!(arr[2]["value"], serde_json::Value::Null);
    }

    #[test]
    fn format_groups_md_basic() {
        let groups = vec![RecordGroup {
            key: "active".into(),
            value: QValue::Text("active".into()),
            count: 2,
            paths: vec!["a.md".into(), "b.md".into()],
        }];
        let md = format_groups_md(&groups);
        assert!(md.contains("| active | 2 | a.md, b.md |"));
        assert!(md.contains("| Group | Count |"));
    }

    #[test]
    fn group_by_with_query_filter_integration() {
        // Demonstrate that GROUP BY works with query_records output:
        // first filter via a Query, then group the matched records.
        let records = [
            rec("a.md")
                .with_prop("status", QValue::Text("active".into()))
                .with_prop("priority", QValue::Number(3.0)),
            rec("b.md")
                .with_prop("status", QValue::Text("active".into()))
                .with_prop("priority", QValue::Number(5.0)),
            rec("c.md")
                .with_prop("status", QValue::Text("done".into()))
                .with_prop("priority", QValue::Number(1.0)),
        ];
        // Filter to only active records, then group by priority.
        let q = Query {
            filter: Condition::Cmp {
                field: "status".into(),
                op: CmpOp::Eq,
                value: Operand::Literal(QValue::Text("active".into())),
            },
            ..Default::default()
        };
        let matched: Vec<Record> = records
            .iter()
            .filter(|r| eval_cond(&q.filter, r))
            .cloned()
            .collect();
        let groups = group_records_by(&matched, "priority");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].key, "3");
        assert_eq!(groups[1].key, "5");
    }
}
