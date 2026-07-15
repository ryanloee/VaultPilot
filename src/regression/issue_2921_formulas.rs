//! Regression tests for #2921 — vault-query Formulas (computed columns).
//!
//! Verifies formula parsing, evaluation, and integration with query_records.

use crate::vault_query::{
    parse_formula_expr, parse_formula_spec, query_records, Direction, Formula, FormulaExpr, QValue,
    Query, Record,
};

/// Helper: create a record with numeric properties.
fn num_rec(path: &str, fields: &[(&str, f64)]) -> Record {
    let mut r = Record::new(path);
    for (k, v) in fields {
        r.props.insert(k.to_string(), QValue::Number(*v));
    }
    r
}

/// Helper: create a record with string properties.
fn text_rec(path: &str, fields: &[(&str, &str)]) -> Record {
    let mut r = Record::new(path);
    for (k, v) in fields {
        r.props.insert(k.to_string(), QValue::Text(v.to_string()));
    }
    r
}

// ── Formula Parsing ─────────────────────────────────────────────────────────

#[test]
fn parse_simple_arithmetic() {
    let expr = parse_formula_expr("a + b * 2").unwrap();
    // Should parse as a + (b * 2)
    match &expr {
        FormulaExpr::Add(left, right) => {
            assert!(matches!(**left, FormulaExpr::Column(ref c) if c == "a"));
            assert!(matches!(**right, FormulaExpr::Mul(..)));
        }
        _ => panic!("expected Add, got {expr:?}"),
    }
}

#[test]
fn parse_negation() {
    let expr = parse_formula_expr("-priority").unwrap();
    match &expr {
        FormulaExpr::Neg(inner) => {
            assert!(matches!(**inner, FormulaExpr::Column(ref c) if c == "priority"));
        }
        _ => panic!("expected Neg, got {expr:?}"),
    }
}

#[test]
fn parse_parenthesized() {
    let expr = parse_formula_expr("(a + b) * 2").unwrap();
    match &expr {
        FormulaExpr::Mul(left, right) => {
            assert!(matches!(**left, FormulaExpr::Add(..)));
            assert!(matches!(**right, FormulaExpr::Number(n) if (n - 2.0).abs() < 0.001));
        }
        _ => panic!("expected Mul, got {expr:?}"),
    }
}

#[test]
fn parse_concat() {
    let expr = parse_formula_expr("concat(first, \" \", last)").unwrap();
    match &expr {
        FormulaExpr::Concat { left, sep, right } => {
            assert!(matches!(**left, FormulaExpr::Column(ref c) if c == "first"));
            assert!(matches!(**sep, FormulaExpr::Text(ref s) if s == " "));
            assert!(matches!(**right, FormulaExpr::Column(ref c) if c == "last"));
        }
        _ => panic!("expected Concat, got {expr:?}"),
    }
}

#[test]
fn parse_if_conditional() {
    // Formula parser doesn't have comparison operators (==, !=, etc.) — those
    // are SQL-level. In formulas, `if(cond, then, else)` uses truthiness:
    // non-zero numbers, non-empty strings, true booleans are truthy.
    // So `if(priority > 3, "high", "low")` isn't supported; use
    // `if(priority, "high", "low")` with SQL WHERE filtering instead.
    let expr = parse_formula_expr("if(priority, 10, 0)").unwrap();
    match &expr {
        FormulaExpr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            assert!(matches!(**cond, FormulaExpr::Column(ref c) if c == "priority"));
            assert!(matches!(
                **then_branch,
                FormulaExpr::Number(n) if (n - 10.0).abs() < 0.001
            ));
            assert!(matches!(
                **else_branch,
                FormulaExpr::Number(n) if (n - 0.0).abs() < 0.001
            ));
        }
        _ => panic!("expected If, got {expr:?}"),
    }
}

#[test]
fn parse_if_with_column_truthiness() {
    let expr = parse_formula_expr("if(priority, 10, 0)").unwrap();
    match &expr {
        FormulaExpr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            assert!(matches!(**cond, FormulaExpr::Column(ref c) if c == "priority"));
            assert!(matches!(**then_branch, FormulaExpr::Number(n) if (n - 10.0).abs() < 0.001));
            assert!(matches!(
                **else_branch,
                FormulaExpr::Number(n) if (n - 0.0).abs() < 0.001
            ));
        }
        _ => panic!("expected If, got {expr:?}"),
    }
}

#[test]
fn parse_datediff() {
    let expr = parse_formula_expr("datediff(due, created)").unwrap();
    match &expr {
        FormulaExpr::DateDiff { end, start } => {
            assert!(matches!(**end, FormulaExpr::Column(ref c) if c == "due"));
            assert!(matches!(**start, FormulaExpr::Column(ref c) if c == "created"));
        }
        _ => panic!("expected DateDiff, got {expr:?}"),
    }
}

#[test]
fn parse_formula_spec_colon_eq() {
    let formula = parse_formula_spec("duration=end - start").unwrap();
    assert_eq!(formula.name, "duration");
    match &formula.expr {
        FormulaExpr::Sub(..) => {}
        _ => panic!("expected Sub"),
    }
}

#[test]
fn parse_formula_spec_rejects_empty_name() {
    assert!(parse_formula_spec("=expr").is_err());
}

#[test]
fn parse_formula_spec_rejects_empty_expr() {
    assert!(parse_formula_spec("name=").is_err());
}

// ── Formula Evaluation ──────────────────────────────────────────────────────

#[test]
fn eval_arithmetic_addition() {
    let rec = num_rec("a.md", &[("x", 3.0), ("y", 5.0)]);
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "sum".into(),
            expr: parse_formula_expr("x + y").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    assert_eq!(rows.len(), 1);
    let val = rows[0].get("sum").unwrap();
    assert_eq!(val, &QValue::Number(8.0));
}

#[test]
fn eval_multiplication() {
    let rec = num_rec("a.md", &[("price", 10.0), ("qty", 3.0)]);
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "total".into(),
            expr: parse_formula_expr("price * qty").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    assert_eq!(rows[0].get("total"), Some(&QValue::Number(30.0)));
}

#[test]
fn eval_division() {
    let rec = num_rec("a.md", &[("total", 100.0), ("count", 5.0)]);
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "avg".into(),
            expr: parse_formula_expr("total / count").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    assert_eq!(rows[0].get("avg"), Some(&QValue::Number(20.0)));
}

#[test]
fn eval_division_by_zero_returns_null() {
    let rec = num_rec("a.md", &[("total", 100.0), ("count", 0.0)]);
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "avg".into(),
            expr: parse_formula_expr("total / count").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    assert_eq!(rows[0].get("avg"), Some(&QValue::Null));
}

#[test]
fn eval_negation() {
    let rec = num_rec("a.md", &[("value", 42.0)]);
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "neg".into(),
            expr: parse_formula_expr("-value").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    assert_eq!(rows[0].get("neg"), Some(&QValue::Number(-42.0)));
}

#[test]
fn eval_concat_with_space_separator() {
    let rec = text_rec("a.md", &[("first", "Alice"), ("last", "Smith")]);
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "full".into(),
            expr: parse_formula_expr("concat(first, \" \", last)").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    assert_eq!(rows[0].get("full"), Some(&QValue::Text("Alice Smith".into())));
}

#[test]
fn eval_upper_lower() {
    let rec = text_rec("a.md", &[("name", "vaultPilot")]);
    let q = Query {
        select: None,
        formulas: vec![
            Formula {
                name: "up".into(),
                expr: parse_formula_expr("upper(name)").unwrap(),
            },
            Formula {
                name: "low".into(),
                expr: parse_formula_expr("lower(name)").unwrap(),
            },
        ],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    assert_eq!(rows[0].get("up"), Some(&QValue::Text("VAULTPILOT".into())));
    assert_eq!(
        rows[0].get("low"),
        Some(&QValue::Text("vaultpilot".into()))
    );
}

#[test]
fn eval_if_truthiness_number() {
    let rec = num_rec("a.md", &[("score", 5.0)]);
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "bonus".into(),
            expr: parse_formula_expr("if(score, 10, 0)").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    // score=5 is truthy (non-zero) → then branch = 10
    assert_eq!(rows[0].get("bonus"), Some(&QValue::Number(10.0)));
}

#[test]
fn eval_if_falsy_zero() {
    let rec = num_rec("a.md", &[("score", 0.0)]);
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "bonus".into(),
            expr: parse_formula_expr("if(score, 10, 0)").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    // score=0 is falsy → else branch = 0
    assert_eq!(rows[0].get("bonus"), Some(&QValue::Number(0.0)));
}

#[test]
fn eval_if_falsy_null_column() {
    let rec = Record::new("a.md"); // no properties
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "check".into(),
            expr: parse_formula_expr("if(missing, \"yes\", \"no\")").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    // missing column → Null → falsy → else branch
    assert_eq!(rows[0].get("check"), Some(&QValue::Text("no".into())));
}

#[test]
fn eval_datediff_in_days() {
    use chrono::NaiveDate;
    let mut rec = Record::new("a.md");
    rec.props.insert(
        "start".into(),
        QValue::Date(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()),
    );
    rec.props.insert(
        "end".into(),
        QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
    );
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "duration".into(),
            expr: parse_formula_expr("datediff(end, start)").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    assert_eq!(rows[0].get("duration"), Some(&QValue::Number(30.0)));
}

#[test]
fn eval_date_subtraction() {
    use chrono::NaiveDate;
    let mut rec = Record::new("a.md");
    rec.props.insert(
        "due".into(),
        QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 15).unwrap()),
    );
    rec.props.insert(
        "created".into(),
        QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
    );
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "days".into(),
            expr: parse_formula_expr("due - created").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    assert_eq!(rows[0].get("days"), Some(&QValue::Number(14.0)));
}

#[test]
fn eval_dateadd() {
    use chrono::NaiveDate;
    let mut rec = Record::new("a.md");
    rec.props.insert(
        "start".into(),
        QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
    );
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "deadline".into(),
            expr: parse_formula_expr("dateadd(start, 30)").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    assert_eq!(
        rows[0].get("deadline"),
        Some(&QValue::Date(
            NaiveDate::from_ymd_opt(2026, 7, 31).unwrap()
        ))
    );
}

#[test]
fn eval_null_column_returns_null() {
    let rec = num_rec("a.md", &[("x", 3.0)]); // no "y"
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "sum".into(),
            expr: parse_formula_expr("x + y").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    // y is Null → Number + Null = Null
    assert_eq!(rows[0].get("sum"), Some(&QValue::Null));
}

// ── Integration: formulas with SELECT projection ────────────────────────────

#[test]
fn formula_works_when_source_col_omitted_from_select() {
    let rec = num_rec("a.md", &[("x", 3.0), ("y", 5.0)]);
    let q = Query {
        select: Some(vec!["x".into()]),
        formulas: vec![Formula {
            name: "sum".into(),
            expr: parse_formula_expr("x + y").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    // Row should have $path, x (3), sum (8) — but NOT y
    assert!(rows[0].contains_key("$path"));
    assert_eq!(rows[0].get("x"), Some(&QValue::Number(3.0)));
    assert_eq!(rows[0].get("sum"), Some(&QValue::Number(8.0)));
    assert!(!rows[0].contains_key("y"), "y should not appear when omitted");
}

// ── Integration: ORDER BY formula column ────────────────────────────────────

#[test]
fn order_by_formula_column() {
    let a = num_rec("a.md", &[("x", 3.0), ("y", 5.0)]);
    let b = num_rec("b.md", &[("x", 1.0), ("y", 2.0)]);
    let q = Query {
        select: None,
        order_by: Some(("sum".into(), Direction::Asc)),
        formulas: vec![Formula {
            name: "sum".into(),
            expr: parse_formula_expr("x + y").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[a.clone(), b.clone()], &q);
    // b (1+2=3) should come first, then a (3+5=8)
    assert_eq!(rows.len(), 2);
    let first_path = rows[0].get("$path").unwrap();
    let second_path = rows[1].get("$path").unwrap();
    assert_eq!(*first_path, QValue::Text("b.md".into()));
    assert_eq!(*second_path, QValue::Text("a.md".into()));
}

// ── Integration: multiple formulas ──────────────────────────────────────────

#[test]
fn multiple_formulas_per_row() {
    let rec = num_rec("a.md", &[("x", 10.0), ("y", 5.0)]);
    let q = Query {
        select: None,
        formulas: vec![
            Formula {
                name: "sum".into(),
                expr: parse_formula_expr("x + y").unwrap(),
            },
            Formula {
                name: "prod".into(),
                expr: parse_formula_expr("x * y").unwrap(),
            },
            Formula {
                name: "diff".into(),
                expr: parse_formula_expr("x - y").unwrap(),
            },
        ],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    assert_eq!(rows[0].get("sum"), Some(&QValue::Number(15.0)));
    assert_eq!(rows[0].get("prod"), Some(&QValue::Number(50.0)));
    assert_eq!(rows[0].get("diff"), Some(&QValue::Number(5.0)));
}

// ── Formula parser edge cases ───────────────────────────────────────────────

#[test]
fn formula_rejects_unknown_function() {
    assert!(parse_formula_expr("unknown(a)").is_err());
}

#[test]
fn formula_rejects_empty_input() {
    assert!(parse_formula_expr("").is_err());
}

#[test]
fn formula_accepts_numeric_literal() {
    let expr = parse_formula_expr("42.5").unwrap();
    assert!(matches!(expr, FormulaExpr::Number(n) if (n - 42.5).abs() < 0.001));
}

#[test]
fn formula_accepts_string_literal() {
    let expr = parse_formula_expr("\"hello\"").unwrap();
    assert_eq!(expr, FormulaExpr::Text("hello".into()));
}

#[test]
fn formula_date_literal() {
    use chrono::NaiveDate;
    let expr = parse_formula_expr("\"2026-07-16\"").unwrap();
    assert_eq!(
        expr,
        FormulaExpr::Date(NaiveDate::from_ymd_opt(2026, 7, 16).unwrap())
    );
}

#[test]
fn formula_add_number_and_text_concat() {
    // "n + \" items\"" should produce "42 items"
    // concat(n, " items") won't work because concat requires 3 args (left, sep, right).
    // Use Add operator for Number+Text concatenation instead.

    let mut r = Record::new("a.md");
    r.props.insert("n".into(), QValue::Number(42.0));

    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "label".into(),
            expr: parse_formula_expr("n + \" items\"").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[r.clone()], &q);
    assert_eq!(
        rows[0].get("label"),
        Some(&QValue::Text("42 items".into()))
    );
}

#[test]
fn formula_text_plus_number() {
    let rec = text_rec("a.md", &[("tag", "v")]);
    let q = Query {
        select: None,
        formulas: vec![Formula {
            name: "version".into(),
            expr: parse_formula_expr("tag + 2").unwrap(),
        }],
        ..Default::default()
    };
    let rows = query_records(&[rec], &q);
    assert_eq!(
        rows[0].get("version"),
        Some(&QValue::Text("v2".into()))
    );
}