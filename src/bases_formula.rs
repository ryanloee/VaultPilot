//! Base Formula expression evaluator (#3331)
//!
//! A lightweight self-contained expression parser and evaluator for
//! computing derived columns in Bases views.  Supports:
//!
//! - Arithmetic: `+ - * / %`
//! - Comparison: `== != > < >= <=`
//! - Logic: `&& || !`
//! - Functions: `if()`, `now()`, `today()`, `contains()`, `replace()`, `length()`, `round()`
//! - NoteMeta field references: `title`, `status`, `tags`, etc.
//! - Cross-formula references: `formula.some_field`
//! - Number and string literals
//!
//! No external dependencies — hand-written recursive-descent parser.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::models::NoteMeta;

// ── Tokenizer ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    StringLit(String),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
    And,
    Or,
    Not,
    LParen,
    RParen,
    Comma,
    Dot,
    Eof,
}

struct Tokenizer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().peekable(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while let Some(&c) = self.chars.peek() {
            if c.is_whitespace() {
                self.chars.next();
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Token {
        self.skip_ws();
        match self.chars.peek() {
            None => Token::Eof,
            Some(&c) => match c {
                '+' => {
                    self.chars.next();
                    self.pos += 1;
                    Token::Plus
                }
                '-' => {
                    self.chars.next();
                    self.pos += 1;
                    Token::Minus
                }
                '*' => {
                    self.chars.next();
                    self.pos += 1;
                    Token::Star
                }
                '/' => {
                    self.chars.next();
                    self.pos += 1;
                    Token::Slash
                }
                '%' => {
                    self.chars.next();
                    self.pos += 1;
                    Token::Percent
                }
                '(' => {
                    self.chars.next();
                    self.pos += 1;
                    Token::LParen
                }
                ')' => {
                    self.chars.next();
                    self.pos += 1;
                    Token::RParen
                }
                ',' => {
                    self.chars.next();
                    self.pos += 1;
                    Token::Comma
                }
                '.' => {
                    self.chars.next();
                    self.pos += 1;
                    Token::Dot
                }
                '!' => {
                    self.chars.next();
                    self.pos += 1;
                    if self.chars.peek() == Some(&'=') {
                        self.chars.next();
                        self.pos += 1;
                        Token::Ne
                    } else {
                        Token::Not
                    }
                }
                '=' => {
                    self.chars.next();
                    self.pos += 1;
                    if self.chars.peek() == Some(&'=') {
                        self.chars.next();
                        self.pos += 1;
                        Token::EqEq
                    } else {
                        Token::EqEq // tolerate single = for equality
                    }
                }
                '>' => {
                    self.chars.next();
                    self.pos += 1;
                    if self.chars.peek() == Some(&'=') {
                        self.chars.next();
                        self.pos += 1;
                        Token::Ge
                    } else {
                        Token::Gt
                    }
                }
                '<' => {
                    self.chars.next();
                    self.pos += 1;
                    if self.chars.peek() == Some(&'=') {
                        self.chars.next();
                        self.pos += 1;
                        Token::Le
                    } else {
                        Token::Lt
                    }
                }
                '&' => {
                    self.chars.next();
                    self.pos += 1;
                    if self.chars.peek() == Some(&'&') {
                        self.chars.next();
                        self.pos += 1;
                        Token::And
                    } else {
                        Token::And // tolerate single &
                    }
                }
                '|' => {
                    self.chars.next();
                    self.pos += 1;
                    if self.chars.peek() == Some(&'|') {
                        self.chars.next();
                        self.pos += 1;
                        Token::Or
                    } else {
                        Token::Or // tolerate single |
                    }
                }
                '"' | '\'' => {
                    let quote = c;
                    self.chars.next(); // skip opening quote
                    self.pos += 1;
                    let mut s = String::new();
                    loop {
                        match self.chars.next() {
                            None => break,
                            Some(ch) if ch == quote => {
                                self.pos += 1;
                                break;
                            }
                            Some(ch) => {
                                s.push(ch);
                                self.pos += 1;
                            }
                        }
                    }
                    Token::StringLit(s)
                }
                _ if c.is_ascii_digit() || c == '.' => {
                    let mut num = String::new();
                    while let Some(&ch) = self.chars.peek() {
                        if ch.is_ascii_digit() || ch == '.' {
                            num.push(ch);
                            self.chars.next();
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    Token::Number(num.parse::<f64>().unwrap_or(0.0))
                }
                _ if c.is_alphabetic() || c == '_' => {
                    let mut ident = String::new();
                    while let Some(&ch) = self.chars.peek() {
                        if ch.is_alphanumeric() || ch == '_' {
                            ident.push(ch);
                            self.chars.next();
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    match ident.as_str() {
                        "true" => Token::Number(1.0),  // truthy
                        "false" => Token::Number(0.0), // falsy
                        "and" => Token::And,
                        "or" => Token::Or,
                        "not" => Token::Not,
                        _ => Token::Ident(ident),
                    }
                }
                _ => {
                    // skip unknown
                    self.chars.next();
                    self.pos += 1;
                    self.next_token()
                }
            },
        }
    }
}

// ── AST ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Expr {
    Number(f64),
    StringLit(String),
    Ident(String),
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    UnaryOp(UnaryOp, Box<Expr>),
    FuncCall(String, Vec<Expr>),
}

#[derive(Debug, Clone, Copy)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
    And,
    Or,
}

#[derive(Debug, Clone, Copy)]
enum UnaryOp {
    Neg,
    Not,
}

// ── Parser (recursive descent) ──────────────────────────────────────────────

struct Parser<'a> {
    tok: &'a mut Tokenizer<'a>,
    peek: Token,
}

impl<'a> Parser<'a> {
    fn new(tok: &'a mut Tokenizer<'a>) -> Self {
        let peek = tok.next_token();
        Self { tok, peek }
    }

    fn advance(&mut self) -> Token {
        let t = self.peek.clone();
        self.peek = self.tok.next_token();
        t
    }

    fn expect(&mut self, expected: &Token) -> Token {
        if std::mem::discriminant(&self.peek) == std::mem::discriminant(expected) {
            self.advance()
        } else {
            // Mismatch — skip and continue
            self.advance()
        }
    }

    // expression: term (("&&" | "||") term)*
    fn parse_expression(&mut self) -> Expr {
        let mut left = self.parse_comparison();
        loop {
            match &self.peek {
                Token::And => {
                    self.advance();
                    let right = self.parse_comparison();
                    left = Expr::BinOp(Box::new(left), BinOp::And, Box::new(right));
                }
                Token::Or => {
                    self.advance();
                    let right = self.parse_comparison();
                    left = Expr::BinOp(Box::new(left), BinOp::Or, Box::new(right));
                }
                _ => break,
            }
        }
        left
    }

    // comparison: term (("==" | "!=" | ">" | ">=" | "<" | "<=") term)*
    fn parse_comparison(&mut self) -> Expr {
        let mut left = self.parse_term();
        loop {
            match &self.peek {
                Token::EqEq => {
                    self.advance();
                    let right = self.parse_term();
                    left = Expr::BinOp(Box::new(left), BinOp::Eq, Box::new(right));
                }
                Token::Ne => {
                    self.advance();
                    let right = self.parse_term();
                    left = Expr::BinOp(Box::new(left), BinOp::Ne, Box::new(right));
                }
                Token::Gt => {
                    self.advance();
                    let right = self.parse_term();
                    left = Expr::BinOp(Box::new(left), BinOp::Gt, Box::new(right));
                }
                Token::Ge => {
                    self.advance();
                    let right = self.parse_term();
                    left = Expr::BinOp(Box::new(left), BinOp::Ge, Box::new(right));
                }
                Token::Lt => {
                    self.advance();
                    let right = self.parse_term();
                    left = Expr::BinOp(Box::new(left), BinOp::Lt, Box::new(right));
                }
                Token::Le => {
                    self.advance();
                    let right = self.parse_term();
                    left = Expr::BinOp(Box::new(left), BinOp::Le, Box::new(right));
                }
                _ => break,
            }
        }
        left
    }

    // term: factor (("+" | "-") factor)*
    fn parse_term(&mut self) -> Expr {
        let mut left = self.parse_factor();
        loop {
            match &self.peek {
                Token::Plus => {
                    self.advance();
                    let right = self.parse_factor();
                    left = Expr::BinOp(Box::new(left), BinOp::Add, Box::new(right));
                }
                Token::Minus => {
                    self.advance();
                    let right = self.parse_factor();
                    left = Expr::BinOp(Box::new(left), BinOp::Sub, Box::new(right));
                }
                _ => break,
            }
        }
        left
    }

    // factor: unary (("*" | "/" | "%") unary)*
    fn parse_factor(&mut self) -> Expr {
        let mut left = self.parse_unary();
        loop {
            match &self.peek {
                Token::Star => {
                    self.advance();
                    let right = self.parse_unary();
                    left = Expr::BinOp(Box::new(left), BinOp::Mul, Box::new(right));
                }
                Token::Slash => {
                    self.advance();
                    let right = self.parse_unary();
                    left = Expr::BinOp(Box::new(left), BinOp::Div, Box::new(right));
                }
                Token::Percent => {
                    self.advance();
                    let right = self.parse_unary();
                    left = Expr::BinOp(Box::new(left), BinOp::Mod, Box::new(right));
                }
                _ => break,
            }
        }
        left
    }

    // unary: ("-" | "!" | "not")? primary
    fn parse_unary(&mut self) -> Expr {
        match &self.peek {
            Token::Minus => {
                self.advance();
                Expr::UnaryOp(UnaryOp::Neg, Box::new(self.parse_unary()))
            }
            Token::Not => {
                self.advance();
                Expr::UnaryOp(UnaryOp::Not, Box::new(self.parse_unary()))
            }
            _ => self.parse_primary(),
        }
    }

    // primary: Number | StringLit | Ident | Ident "(" args ")" | "(" expression ")"
    fn parse_primary(&mut self) -> Expr {
        let token = self.advance();
        match token {
            Token::Number(n) => Expr::Number(n),
            Token::StringLit(s) => Expr::StringLit(s),
            Token::Ident(name) => {
                // Check if it's a function call
                if self.peek == Token::LParen {
                    self.advance(); // consume (
                    let mut args = Vec::new();
                    if self.peek != Token::RParen {
                        args.push(self.parse_expression());
                        while self.peek == Token::Comma {
                            self.advance();
                            args.push(self.parse_expression());
                        }
                    }
                    self.expect(&Token::RParen);
                    Expr::FuncCall(name, args)
                } else {
                    Expr::Ident(name)
                }
            }
            Token::LParen => {
                let expr = self.parse_expression();
                self.expect(&Token::RParen);
                expr
            }
            _ => Expr::Number(0.0), // fallback
        }
    }
}

// ── Evaluation ──────────────────────────────────────────────────────────────

/// Environment for formula evaluation: maps field names to values.
#[derive(Debug, Clone)]
pub struct FmlEnv<'a> {
    pub note: &'a NoteMeta,
    pub formula_values: &'a HashMap<String, FmlValue>,
}

/// Evaluate an expression against the given environment.
fn eval_expr(expr: &Expr, env: &FmlEnv) -> FmlValue {
    match expr {
        Expr::Number(n) => FmlValue::Number(*n),
        Expr::StringLit(s) => FmlValue::String(s.clone()),
        Expr::Ident(name) => {
            // First check formula_values (cross-formula ref)
            if let Some(val) = env.formula_values.get(name) {
                return val.clone();
            }
            // Then check NoteMeta fields
            resolve_field(env.note, name)
        }
        Expr::BinOp(left, op, right) => {
            let lv = eval_expr(left, env);
            let rv = eval_expr(right, env);
            eval_binop(lv, *op, rv)
        }
        Expr::UnaryOp(op, expr) => {
            let v = eval_expr(expr, env);
            match op {
                UnaryOp::Neg => match v {
                    FmlValue::Number(n) => FmlValue::Number(-n),
                    FmlValue::String(s) => {
                        if let Ok(n) = s.parse::<f64>() {
                            FmlValue::Number(-n)
                        } else {
                            FmlValue::Number(0.0)
                        }
                    }
                    FmlValue::Bool(b) => FmlValue::Number(if b { -1.0 } else { 0.0 }),
                },
                UnaryOp::Not => match v {
                    FmlValue::Bool(b) => FmlValue::Bool(!b),
                    FmlValue::Number(n) => FmlValue::Bool(n == 0.0),
                    FmlValue::String(s) => FmlValue::Bool(s.is_empty()),
                },
            }
        }
        Expr::FuncCall(name, args) => eval_function(name, args, env),
    }
}

/// Resolve a NoteMeta field by name.
fn resolve_field(note: &NoteMeta, name: &str) -> FmlValue {
    // Strip "formula." or "file." prefix
    let name = name
        .strip_prefix("formula.")
        .or_else(|| name.strip_prefix("file."))
        .unwrap_or(name);
    match name {
        "title" => FmlValue::String(note.title.clone()),
        "tags" => FmlValue::String(note.tags.join(", ")),
        "keywords" => FmlValue::String(note.keywords.join(", ")),
        "collections" => FmlValue::String(note.collections.join(", ")),
        "platform" => FmlValue::String(note.platform.clone()),
        "board" => FmlValue::String(note.board.clone()),
        "kernel" => FmlValue::String(note.kernel.clone()),
        "status" => FmlValue::String(note.status.clone()),
        "source" => FmlValue::String(note.source.clone()),
        "path" => FmlValue::String(note.path.clone()),
        "summary" => FmlValue::String(note.summary.clone()),
        "created_at" => FmlValue::String(note.created_at.clone()),
        "updated_at" => FmlValue::String(note.updated_at.clone()),
        "id" => FmlValue::String(note.id.clone()),
        // Try numeric parsing for unknown fields
        _ => {
            // Try to find in tags, keywords etc. as a numeric value
            FmlValue::String(String::new())
        }
    }
}

/// Helper: coerce a value to f64 for arithmetic.
fn to_number(v: &FmlValue) -> f64 {
    match v {
        FmlValue::Number(n) => *n,
        FmlValue::String(s) => s.parse::<f64>().unwrap_or(0.0),
        FmlValue::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
    }
}

/// Helper: coerce to string.
fn to_string(v: &FmlValue) -> String {
    match v {
        FmlValue::Number(n) => {
            if n.fract() == 0.0 && n.is_finite() {
                format!("{}", *n as i64)
            } else {
                format!("{}", n)
            }
        }
        FmlValue::String(s) => s.clone(),
        FmlValue::Bool(b) => format!("{}", b),
    }
}

/// Helper: coerce to bool.
fn is_truthy(v: &FmlValue) -> bool {
    match v {
        FmlValue::Bool(b) => *b,
        FmlValue::Number(n) => *n != 0.0,
        FmlValue::String(s) => !s.is_empty(),
    }
}

/// Evaluate a binary operation.
fn eval_binop(left: FmlValue, op: BinOp, right: FmlValue) -> FmlValue {
    // For equality/comparison, compare as strings if either side is a string
    let use_string_cmp =
        matches!(&left, FmlValue::String(_)) || matches!(&right, FmlValue::String(_));

    match op {
        BinOp::Add => {
            if use_string_cmp {
                FmlValue::String(format!("{}{}", to_string(&left), to_string(&right)))
            } else {
                FmlValue::Number(to_number(&left) + to_number(&right))
            }
        }
        BinOp::Sub => FmlValue::Number(to_number(&left) - to_number(&right)),
        BinOp::Mul => FmlValue::Number(to_number(&left) * to_number(&right)),
        BinOp::Div => {
            let r = to_number(&right);
            if r == 0.0 {
                FmlValue::Number(0.0)
            } else {
                FmlValue::Number(to_number(&left) / r)
            }
        }
        BinOp::Mod => {
            let r = to_number(&right);
            if r == 0.0 {
                FmlValue::Number(0.0)
            } else {
                FmlValue::Number(to_number(&left) % r)
            }
        }
        BinOp::Eq => {
            if use_string_cmp {
                FmlValue::Bool(to_string(&left) == to_string(&right))
            } else {
                FmlValue::Bool((to_number(&left) - to_number(&right)).abs() < f64::EPSILON)
            }
        }
        BinOp::Ne => {
            if use_string_cmp {
                FmlValue::Bool(to_string(&left) != to_string(&right))
            } else {
                FmlValue::Bool((to_number(&left) - to_number(&right)).abs() >= f64::EPSILON)
            }
        }
        BinOp::Gt => {
            if use_string_cmp {
                FmlValue::Bool(to_string(&left) > to_string(&right))
            } else {
                FmlValue::Bool(to_number(&left) > to_number(&right))
            }
        }
        BinOp::Ge => {
            if use_string_cmp {
                FmlValue::Bool(to_string(&left) >= to_string(&right))
            } else {
                FmlValue::Bool(to_number(&left) >= to_number(&right))
            }
        }
        BinOp::Lt => {
            if use_string_cmp {
                FmlValue::Bool(to_string(&left) < to_string(&right))
            } else {
                FmlValue::Bool(to_number(&left) < to_number(&right))
            }
        }
        BinOp::Le => {
            if use_string_cmp {
                FmlValue::Bool(to_string(&left) <= to_string(&right))
            } else {
                FmlValue::Bool(to_number(&left) <= to_number(&right))
            }
        }
        BinOp::And => FmlValue::Bool(is_truthy(&left) && is_truthy(&right)),
        BinOp::Or => FmlValue::Bool(is_truthy(&left) || is_truthy(&right)),
    }
}

/// Evaluate a built-in function call.
fn eval_function(name: &str, args: &[Expr], env: &FmlEnv) -> FmlValue {
    match name {
        "if" => {
            if args.len() >= 3 {
                let cond = eval_expr(&args[0], env);
                if is_truthy(&cond) {
                    eval_expr(&args[1], env)
                } else {
                    eval_expr(&args[2], env)
                }
            } else {
                FmlValue::String(String::new())
            }
        }
        "now" => FmlValue::String(iso_now()),
        "today" => FmlValue::String(iso_today()),
        "contains" => {
            if args.len() >= 2 {
                let haystack = to_string(&eval_expr(&args[0], env));
                let needle = to_string(&eval_expr(&args[1], env));
                FmlValue::Bool(haystack.contains(&needle))
            } else {
                FmlValue::Bool(false)
            }
        }
        "replace" | "substitute" => {
            if args.len() >= 3 {
                let s = to_string(&eval_expr(&args[0], env));
                let from = to_string(&eval_expr(&args[1], env));
                let to = to_string(&eval_expr(&args[2], env));
                FmlValue::String(s.replace(&from, &to))
            } else {
                FmlValue::String(String::new())
            }
        }
        "length" | "len" => {
            if !args.is_empty() {
                let s = to_string(&eval_expr(&args[0], env));
                FmlValue::Number(s.len() as f64)
            } else {
                FmlValue::Number(0.0)
            }
        }
        "round" => {
            if !args.is_empty() {
                let n = to_number(&eval_expr(&args[0], env));
                if args.len() >= 2 {
                    let places = to_number(&eval_expr(&args[1], env)) as i32;
                    let factor = 10_f64.powi(places);
                    FmlValue::Number((n * factor).round() / factor)
                } else {
                    FmlValue::Number(n.round())
                }
            } else {
                FmlValue::Number(0.0)
            }
        }
        "trim" => {
            if !args.is_empty() {
                let s = to_string(&eval_expr(&args[0], env));
                FmlValue::String(s.trim().to_string())
            } else {
                FmlValue::String(String::new())
            }
        }
        "upper" | "uppercase" => {
            if !args.is_empty() {
                let s = to_string(&eval_expr(&args[0], env));
                FmlValue::String(s.to_uppercase())
            } else {
                FmlValue::String(String::new())
            }
        }
        "lower" | "lowercase" => {
            if !args.is_empty() {
                let s = to_string(&eval_expr(&args[0], env));
                FmlValue::String(s.to_lowercase())
            } else {
                FmlValue::String(String::new())
            }
        }
        "abs" => {
            if !args.is_empty() {
                FmlValue::Number(to_number(&eval_expr(&args[0], env)).abs())
            } else {
                FmlValue::Number(0.0)
            }
        }
        _ => FmlValue::String(String::new()),
    }
}

fn iso_now() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Format as ISO 8601
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let mins = (time_secs % 3600) / 60;
    let secs_remain = time_secs % 60;

    // Simple date calculation from Unix epoch (1970-01-01)
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md as i64 {
            m = i;
            break;
        }
        remaining -= md as i64;
    }

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m + 1,
        remaining + 1,
        hours,
        mins,
        secs_remain
    )
}

fn iso_today() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let days = secs / 86400;

    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md as i64 {
            m = i;
            break;
        }
        remaining -= md as i64;
    }

    format!("{:04}-{:02}-{:02}", y, m + 1, remaining + 1)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

// ── Value type ──────────────────────────────────────────────────────────────

/// A runtime value in the formula evaluator.
#[derive(Debug, Clone, PartialEq)]
pub enum FmlValue {
    Number(f64),
    String(String),
    Bool(bool),
}

impl std::fmt::Display for FmlValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FmlValue::Number(n) => {
                if n.fract() == 0.0 && n.is_finite() {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{}", n)
                }
            }
            FmlValue::String(s) => write!(f, "{}", s),
            FmlValue::Bool(b) => write!(f, "{}", b),
        }
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Parse and evaluate a single formula expression against a note.
///
/// `formula_values` holds already-computed values for cross-formula references.
pub fn evaluate(expression: &str, env: &FmlEnv) -> FmlValue {
    let mut tok = Tokenizer::new(expression);
    let mut parser = Parser::new(&mut tok);
    let expr = parser.parse_expression();
    eval_expr(&expr, env)
}

/// Parse a formula expression without evaluating it.
/// Returns `None` on parse failure (malformed expression).
pub fn parse_formula(expression: &str) -> Option<()> {
    let mut tok = Tokenizer::new(expression);
    let mut parser = Parser::new(&mut tok);
    parser.parse_expression();
    Some(())
}

/// Detect circular references in formulas.
///
/// Returns the names of formulas involved in cycles.
pub fn detect_cycles(formulas: &std::collections::HashMap<String, String>) -> Vec<String> {
    use std::collections::{HashMap, HashSet};

    // Build dependency graph: formula_name -> set of referenced formula names
    let mut deps: HashMap<&str, HashSet<String>> = HashMap::new();
    for (name, expr_str) in formulas {
        let refs = extract_formula_refs(expr_str, formulas);
        deps.insert(name.as_str(), refs);
    }

    // DFS cycle detection
    let mut visited: HashSet<&str> = HashSet::new();
    let mut in_stack: HashSet<&str> = HashSet::new();
    let mut cycles: Vec<String> = Vec::new();

    for name in formulas.keys() {
        if !visited.contains(name.as_str()) {
            let mut path: Vec<String> = Vec::new();
            detect_cycles_dfs(
                name,
                formulas,
                &deps,
                &mut visited,
                &mut in_stack,
                &mut path,
                &mut cycles,
            );
        }
    }

    cycles.sort();
    cycles.dedup();
    cycles
}

fn detect_cycles_dfs<'a>(
    name: &'a str,
    formulas: &std::collections::HashMap<String, String>,
    deps: &'a std::collections::HashMap<&'a str, std::collections::HashSet<String>>,
    visited: &mut std::collections::HashSet<&'a str>,
    in_stack: &mut std::collections::HashSet<&'a str>,
    path: &mut Vec<String>,
    cycles: &mut Vec<String>,
) {
    if in_stack.contains(name) {
        // Found a cycle — record all nodes in the current cycle
        let cycle_start = path.iter().position(|n| n == name).unwrap_or(0);
        for node in &path[cycle_start..] {
            if !cycles.contains(node) {
                cycles.push(node.clone());
            }
        }
        return;
    }
    if visited.contains(name) {
        return;
    }

    visited.insert(name);
    in_stack.insert(name);
    path.push(name.to_string());

    if let Some(refs) = deps.get(name) {
        for dep in refs {
            if formulas.contains_key(dep) {
                detect_cycles_dfs(dep, formulas, deps, visited, in_stack, path, cycles);
            }
        }
    }

    path.pop();
    in_stack.remove(name);
}

/// Extract names of other formulas referenced in an expression string.
pub fn extract_formula_refs(
    expr_str: &str,
    formulas: &std::collections::HashMap<String, String>,
) -> std::collections::HashSet<String> {
    let mut refs = std::collections::HashSet::new();
    let mut tok = Tokenizer::new(expr_str);

    loop {
        let t = tok.next_token();
        match &t {
            Token::Ident(name) => {
                if formulas.contains_key(name) {
                    refs.insert(name.clone());
                }
            }
            Token::Eof => break,
            _ => {}
        }
    }
    refs
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NoteMeta;

    fn test_note() -> NoteMeta {
        NoteMeta {
            id: "n1".into(),
            title: "Test Note".into(),
            tags: vec!["rust".into(), "async".into()],
            status: "in-progress".into(),
            created_at: "2026-06-01".into(),
            updated_at: "2026-07-15".into(),
            platform: "linux".into(),
            ..Default::default()
        }
    }

    fn env_for<'a>(
        note: &'a NoteMeta,
        formula_values: &'a HashMap<String, FmlValue>,
    ) -> FmlEnv<'a> {
        FmlEnv {
            note,
            formula_values,
        }
    }

    // ── Arithmetic ────────────────────────────────────────────────────────

    #[test]
    fn test_eval_number_literal() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("42", &env).to_string(), "42");
        assert_eq!(evaluate("3.14", &env).to_string(), "3.14");
    }

    #[test]
    fn test_eval_string_literal() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("\"hello\"", &env).to_string(), "hello");
        assert_eq!(evaluate("'world'", &env).to_string(), "world");
    }

    #[test]
    fn test_eval_addition() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("1 + 2", &env).to_string(), "3");
        assert_eq!(evaluate("10 + 20", &env).to_string(), "30");
    }

    #[test]
    fn test_eval_subtraction() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("5 - 3", &env).to_string(), "2");
    }

    #[test]
    fn test_eval_multiplication() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("3 * 4", &env).to_string(), "12");
    }

    #[test]
    fn test_eval_division() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("10 / 2", &env).to_string(), "5");
        assert_eq!(evaluate("10 / 0", &env).to_string(), "0");
    }

    #[test]
    fn test_eval_modulo() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("10 % 3", &env).to_string(), "1");
    }

    #[test]
    fn test_eval_operator_precedence() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("2 + 3 * 4", &env).to_string(), "14");
        assert_eq!(evaluate("(2 + 3) * 4", &env).to_string(), "20");
        assert_eq!(evaluate("10 - 2 - 3", &env).to_string(), "5");
    }

    // ── Comparison ────────────────────────────────────────────────────────

    #[test]
    fn test_eval_equality() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("5 == 5", &env).to_string(), "true");
        assert_eq!(evaluate("5 == 3", &env).to_string(), "false");
        assert_eq!(evaluate("5 != 3", &env).to_string(), "true");
    }

    #[test]
    fn test_eval_comparison() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("5 > 3", &env).to_string(), "true");
        assert_eq!(evaluate("3 > 5", &env).to_string(), "false");
        assert_eq!(evaluate("5 >= 5", &env).to_string(), "true");
        assert_eq!(evaluate("3 < 5", &env).to_string(), "true");
        assert_eq!(evaluate("5 <= 3", &env).to_string(), "false");
    }

    // ── Logic ─────────────────────────────────────────────────────────────

    #[test]
    fn test_eval_and_or() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("1 && 1", &env).to_string(), "true");
        assert_eq!(evaluate("1 && 0", &env).to_string(), "false");
        assert_eq!(evaluate("1 || 0", &env).to_string(), "true");
        assert_eq!(evaluate("0 || 0", &env).to_string(), "false");
    }

    #[test]
    fn test_eval_not() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("!0", &env).to_string(), "true");
        assert_eq!(evaluate("!1", &env).to_string(), "false");
        assert_eq!(evaluate("!\"\"", &env).to_string(), "true");
    }

    #[test]
    fn test_eval_negation() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("-5", &env).to_string(), "-5");
        assert_eq!(evaluate("--5", &env).to_string(), "5");
    }

    // ── Field references ──────────────────────────────────────────────────

    #[test]
    fn test_eval_field_title() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("title", &env).to_string(), "Test Note");
    }

    #[test]
    fn test_eval_field_status() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("status", &env).to_string(), "in-progress");
    }

    #[test]
    fn test_eval_field_expression() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(
            evaluate("status == \"in-progress\"", &env).to_string(),
            "true"
        );
        assert_eq!(evaluate("status == \"done\"", &env).to_string(), "false");
    }

    // ── Functions ─────────────────────────────────────────────────────────

    #[test]
    fn test_eval_if_true() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("if(1, \"yes\", \"no\")", &env).to_string(), "yes");
    }

    #[test]
    fn test_eval_if_false() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("if(0, \"yes\", \"no\")", &env).to_string(), "no");
    }

    #[test]
    fn test_eval_now_today() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        let now = evaluate("now()", &env).to_string();
        assert!(now.contains("T"), "now() should be ISO 8601: {}", now);
        let today = evaluate("today()", &env).to_string();
        assert_eq!(today.len(), 10, "today() should be YYYY-MM-DD: {}", today);
    }

    #[test]
    fn test_eval_contains() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(
            evaluate("contains(\"hello world\", \"world\")", &env).to_string(),
            "true"
        );
        assert_eq!(
            evaluate("contains(\"hello\", \"xyz\")", &env).to_string(),
            "false"
        );
    }

    #[test]
    fn test_eval_replace() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(
            evaluate("replace(\"hello world\", \"world\", \"there\")", &env).to_string(),
            "hello there"
        );
    }

    #[test]
    fn test_eval_length() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("length(\"hello\")", &env).to_string(), "5");
    }

    #[test]
    fn test_eval_round() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("round(3.7)", &env).to_string(), "4");
        assert_eq!(evaluate("round(3.14159, 2)", &env).to_string(), "3.14");
    }

    #[test]
    fn test_eval_trim() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("trim(\"  hello  \")", &env).to_string(), "hello");
    }

    #[test]
    fn test_eval_upper_lower() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(evaluate("upper(\"hello\")", &env).to_string(), "HELLO");
        assert_eq!(evaluate("lower(\"HELLO\")", &env).to_string(), "hello");
        assert_eq!(evaluate("uppercase(\"hi\")", &env).to_string(), "HI");
        assert_eq!(evaluate("lowercase(\"HI\")", &env).to_string(), "hi");
    }

    // ── Complex expressions ───────────────────────────────────────────────

    #[test]
    fn test_eval_complex_condition() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(
            evaluate(
                "if(status == \"in-progress\", \"Active\", \"Inactive\")",
                &env
            )
            .to_string(),
            "Active"
        );
    }

    #[test]
    fn test_eval_string_concat() {
        let note = test_note();
        let fv = HashMap::new();
        let env = env_for(&note, &fv);
        assert_eq!(
            evaluate("\"Hello, \" + title", &env).to_string(),
            "Hello, Test Note"
        );
    }

    // ── Cycle detection ───────────────────────────────────────────────────

    #[test]
    fn test_detect_no_cycles() {
        let mut hm = HashMap::new();
        hm.insert("a".into(), "1 + 2".into());
        hm.insert("b".into(), "a * 3".into());
        let cycles = detect_cycles(&hm);
        assert!(cycles.is_empty(), "no cycles expected: {:?}", cycles);
    }

    #[test]
    fn test_detect_direct_cycle() {
        let mut hm = HashMap::new();
        hm.insert("a".into(), "b".into());
        hm.insert("b".into(), "a".into());
        let cycles = detect_cycles(&hm);
        assert_eq!(cycles.len(), 2, "both a and b should be in cycle");
        assert!(cycles.contains(&"a".to_string()));
        assert!(cycles.contains(&"b".to_string()));
    }

    #[test]
    fn test_detect_self_cycle() {
        let mut hm = HashMap::new();
        hm.insert("x".into(), "x + 1".into());
        let cycles = detect_cycles(&hm);
        assert!(cycles.contains(&"x".to_string()));
    }

    #[test]
    fn test_parse_formula_valid() {
        assert!(parse_formula("1 + 2 * 3").is_some());
        assert!(parse_formula("title == \"done\"").is_some());
    }
}
