//! DBISAM 4 SQL render rules — the pushdown contract for `dbisam_fdw`.
//!
//! Renders the *foldable predicate subset* (the things DBISAM evaluates
//! correctly and cheaply) into a DBISAM `WHERE` expression, and refuses
//! everything else by returning `None` so Postgres applies it as a local
//! recheck. See `proj_init/04-pushdown-contract.md`; the authoritative grammar
//! is **Dibdog** and the reference rules are Delilah's `dbisam_filter_render`.
//!
//! ## Neutral by design
//!
//! The predicate input here ([`Pred`]) is a small neutral AST, deliberately
//! independent of Supabase Wrappers' `Qual`/`Value` types. The extension crate
//! owns a thin adapter (Wrappers qual → [`Pred`]); keeping the rules off the
//! pgrx types means this crate compiles and unit-tests with no Postgres
//! toolchain — which is where the correctness-critical quirk logic wants to
//! live.
//!
//! ## The four dialect quirks (04 §"four dialect quirks")
//!
//! 1. **`TOP n` is trailing** — emitted by [`top_clause`], appended after the
//!    table, never `SELECT TOP n`.
//! 2. **`col <> x` includes NULLs** — `<>` and `NOT IN` are rendered with an
//!    `AND col IS NOT NULL` guard to restore ANSI semantics.
//! 3. **`LIKE` is prefix-only** — only `'abc%'`-shaped patterns are accepted
//!    ([`Pred::LikePrefix`]); the adapter must reject leading/internal `%`.
//! 4. **Other comparisons are ANSI-safe** — `=`,`<`,`>`,`<=`,`>=`,`IN`,
//!    `IS [NOT] NULL` need no compensation.
//!
//! ## Boolean structure
//!
//! - `AND` drops unfoldable conjuncts and pushes the rest (Postgres rechecks
//!   the dropped ones) — partial pushdown is safe under conjunction.
//! - `OR` is all-or-nothing: if *any* disjunct is unfoldable the whole `OR`
//!   falls back, because pushing a subset of disjuncts would wrongly narrow
//!   the result. Correctness over cleverness.

/// A scalar literal appearing on the value side of a predicate.
///
/// Dates/times are intentionally absent: DBISAM's date literal syntax
/// (`#...#`) must be pinned against Dibdog/Derek before we render it, and a
/// wrong date literal returns wrong data, not a slow query. Until then, date
/// predicates arrive as [`Pred::Unsupported`] and Postgres handles them.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Text(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Scalar {
    /// Render as a DBISAM SQL literal. Text is single-quoted with `'` doubled.
    fn render(&self) -> String {
        match self {
            Scalar::Text(s) => format!("'{}'", s.replace('\'', "''")),
            Scalar::Int(n) => n.to_string(),
            Scalar::Float(f) => f.to_string(),
            // DBISAM accepts TRUE/FALSE keyword literals for boolean columns.
            // TODO(dibdog): confirm vs 0/1 before widening boolean pushdown.
            Scalar::Bool(b) => if *b { "TRUE".into() } else { "FALSE".into() },
        }
    }
}

/// The column side of a predicate — a bare column or a whitelisted
/// single-column expression. The whitelist is deliberately tiny; expand it
/// only against Dibdog (04 §"Whitelisted single-column expressions").
#[derive(Debug, Clone, PartialEq)]
pub enum ColExpr {
    /// A bare column reference.
    Col(String),
    /// `LEFT(col, n)` — e.g. `WHERE LEFT(code, 1) IN ('4','6')`.
    Left(String, u32),
}

impl ColExpr {
    fn render(&self) -> String {
        match self {
            ColExpr::Col(c) => c.clone(),
            ColExpr::Left(c, n) => format!("LEFT({c}, {n})"),
        }
    }

    /// The underlying column name, for building NULL guards.
    fn column(&self) -> &str {
        match self {
            ColExpr::Col(c) | ColExpr::Left(c, _) => c,
        }
    }
}

/// Comparison operators in the foldable subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl CmpOp {
    fn sql(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "<>",
            CmpOp::Lt => "<",
            CmpOp::Gt => ">",
            CmpOp::Le => "<=",
            CmpOp::Ge => ">=",
        }
    }
}

/// A predicate node. The adapter maps Postgres quals into this; anything it
/// can't represent becomes [`Pred::Unsupported`].
#[derive(Debug, Clone, PartialEq)]
pub enum Pred {
    /// `col OP value`.
    Compare { left: ColExpr, op: CmpOp, value: Scalar },
    /// `col [NOT] IN (v, ...)`.
    In { col: ColExpr, values: Vec<Scalar>, negated: bool },
    /// `col IS [NOT] NULL`.
    IsNull { col: String, negated: bool },
    /// `col LIKE 'prefix%'`. `prefix` is the literal text *without* the
    /// trailing `%`; it must contain no wildcards (adapter's responsibility).
    LikePrefix { col: String, prefix: String },
    And(Vec<Pred>),
    Or(Vec<Pred>),
    /// Not foldable — Postgres applies it after the scan.
    Unsupported,
}

impl Pred {
    /// Render this predicate to a DBISAM boolean expression, or `None` if it
    /// cannot be pushed (the caller leaves it to Postgres). The returned
    /// string is the bare expression — the caller prepends `WHERE`.
    pub fn render(&self) -> Option<String> {
        match self {
            Pred::Compare { left, op, value } => {
                let base = format!("{} {} {}", left.render(), op.sql(), value.render());
                // Quirk #2: `<>` lets NULL rows through on DBISAM.
                if *op == CmpOp::Ne {
                    Some(format!("({base} AND {} IS NOT NULL)", left.column()))
                } else {
                    Some(base)
                }
            }
            Pred::In { col, values, negated } => {
                if values.is_empty() {
                    return None;
                }
                let list = values.iter().map(Scalar::render).collect::<Vec<_>>().join(", ");
                let kw = if *negated { "NOT IN" } else { "IN" };
                let base = format!("{} {} ({})", col.render(), kw, list);
                // Quirk #2: NOT IN includes NULLs, same as `<>`.
                if *negated {
                    Some(format!("({base} AND {} IS NOT NULL)", col.column()))
                } else {
                    Some(base)
                }
            }
            Pred::IsNull { col, negated } => {
                Some(if *negated {
                    format!("{col} IS NOT NULL")
                } else {
                    format!("{col} IS NULL")
                })
            }
            Pred::LikePrefix { col, prefix } => {
                Some(format!("{col} LIKE '{}%'", prefix.replace('\'', "''")))
            }
            Pred::And(children) => {
                // Push the foldable conjuncts; Postgres rechecks the rest.
                let parts: Vec<String> = children.iter().filter_map(Pred::render).collect();
                match parts.len() {
                    0 => None,
                    1 => Some(parts.into_iter().next().unwrap()),
                    _ => Some(format!("({})", parts.join(" AND "))),
                }
            }
            Pred::Or(children) => {
                // All-or-nothing: a partial OR would wrongly narrow the result.
                let mut parts = Vec::with_capacity(children.len());
                for c in children {
                    parts.push(c.render()?);
                }
                if parts.is_empty() {
                    None
                } else {
                    Some(format!("({})", parts.join(" OR ")))
                }
            }
            Pred::Unsupported => None,
        }
    }
}

/// Render a list of top-level quals (implicitly AND-ed, as Postgres hands
/// them) into a single `WHERE` expression, or `None` if none are foldable.
pub fn render_where(quals: &[Pred]) -> Option<String> {
    Pred::And(quals.to_vec()).render()
}

/// The trailing `TOP n` clause (quirk #1). Append after the table reference:
/// `SELECT ... FROM T WHERE ... TOP n`. The caller decides *whether* it is
/// safe to push — a non-pushable filter sitting above the scan means `TOP n`
/// would cap the wrong row count, so fall back to first-batch sizing instead
/// (04 §"Limit edge case").
pub fn top_clause(n: u64) -> String {
    format!("TOP {n}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(c: &str) -> ColExpr {
        ColExpr::Col(c.into())
    }

    #[test]
    fn ansi_safe_comparisons_have_no_null_guard() {
        for (op, sym) in [
            (CmpOp::Eq, "="),
            (CmpOp::Lt, "<"),
            (CmpOp::Gt, ">"),
            (CmpOp::Le, "<="),
            (CmpOp::Ge, ">="),
        ] {
            let p = Pred::Compare { left: col("qty"), op, value: Scalar::Int(5) };
            assert_eq!(p.render().unwrap(), format!("qty {sym} 5"));
        }
    }

    #[test]
    fn ne_gets_null_guard() {
        let p = Pred::Compare {
            left: col("status"),
            op: CmpOp::Ne,
            value: Scalar::Text("X".into()),
        };
        assert_eq!(p.render().unwrap(), "(status <> 'X' AND status IS NOT NULL)");
    }

    #[test]
    fn not_in_gets_null_guard_in_does_not() {
        let vals = vec![Scalar::Int(1), Scalar::Int(2)];
        let inp = Pred::In { col: col("c"), values: vals.clone(), negated: false };
        assert_eq!(inp.render().unwrap(), "c IN (1, 2)");
        let notin = Pred::In { col: col("c"), values: vals, negated: true };
        assert_eq!(notin.render().unwrap(), "(c NOT IN (1, 2) AND c IS NOT NULL)");
    }

    #[test]
    fn empty_in_is_not_pushed() {
        let p = Pred::In { col: col("c"), values: vec![], negated: false };
        assert_eq!(p.render(), None);
    }

    #[test]
    fn text_literals_escape_quotes() {
        let p = Pred::Compare {
            left: col("name"),
            op: CmpOp::Eq,
            value: Scalar::Text("O'Brien".into()),
        };
        assert_eq!(p.render().unwrap(), "name = 'O''Brien'");
    }

    #[test]
    fn is_null_variants() {
        assert_eq!(Pred::IsNull { col: "c".into(), negated: false }.render().unwrap(), "c IS NULL");
        assert_eq!(Pred::IsNull { col: "c".into(), negated: true }.render().unwrap(), "c IS NOT NULL");
    }

    #[test]
    fn like_is_prefix_only() {
        let p = Pred::LikePrefix { col: "code".into(), prefix: "AB".into() };
        assert_eq!(p.render().unwrap(), "code LIKE 'AB%'");
    }

    #[test]
    fn left_whitelisted_expr() {
        let p = Pred::In {
            col: ColExpr::Left("code".into(), 1),
            values: vec![Scalar::Text("4".into()), Scalar::Text("6".into())],
            negated: false,
        };
        assert_eq!(p.render().unwrap(), "LEFT(code, 1) IN ('4', '6')");
    }

    #[test]
    fn and_drops_unsupported_conjuncts() {
        let p = Pred::And(vec![
            Pred::Compare { left: col("a"), op: CmpOp::Eq, value: Scalar::Int(1) },
            Pred::Unsupported,
            Pred::Compare { left: col("b"), op: CmpOp::Gt, value: Scalar::Int(2) },
        ]);
        assert_eq!(p.render().unwrap(), "(a = 1 AND b > 2)");
    }

    #[test]
    fn and_of_single_foldable_has_no_parens() {
        let p = Pred::And(vec![
            Pred::Unsupported,
            Pred::Compare { left: col("a"), op: CmpOp::Eq, value: Scalar::Int(1) },
        ]);
        assert_eq!(p.render().unwrap(), "a = 1");
    }

    #[test]
    fn and_all_unsupported_is_none() {
        let p = Pred::And(vec![Pred::Unsupported, Pred::Unsupported]);
        assert_eq!(p.render(), None);
    }

    #[test]
    fn or_is_all_or_nothing() {
        let foldable = Pred::Or(vec![
            Pred::Compare { left: col("a"), op: CmpOp::Eq, value: Scalar::Int(1) },
            Pred::Compare { left: col("b"), op: CmpOp::Eq, value: Scalar::Int(2) },
        ]);
        assert_eq!(foldable.render().unwrap(), "(a = 1 OR b = 2)");

        let partial = Pred::Or(vec![
            Pred::Compare { left: col("a"), op: CmpOp::Eq, value: Scalar::Int(1) },
            Pred::Unsupported,
        ]);
        assert_eq!(partial.render(), None);
    }

    #[test]
    fn top_clause_is_trailing_fragment() {
        assert_eq!(top_clause(5), "TOP 5");
    }

    #[test]
    fn render_where_combines_top_level_quals() {
        let quals = vec![
            Pred::Compare { left: col("a"), op: CmpOp::Eq, value: Scalar::Int(1) },
            Pred::Compare { left: col("b"), op: CmpOp::Ne, value: Scalar::Int(2) },
        ];
        assert_eq!(
            render_where(&quals).unwrap(),
            "(a = 1 AND (b <> 2 AND b IS NOT NULL))"
        );
    }
}
