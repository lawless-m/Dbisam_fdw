//! Adapter: Supabase Wrappers [`Qual`] → [`dbisam_sql::Pred`].
//!
//! This is the only place that knows the Wrappers qual vocabulary. The
//! rendering rules and the four DBISAM quirks live in `dbisam-sql` (which is
//! pgrx-free and unit-tested); anything this adapter can't represent becomes
//! [`Pred::Unsupported`], i.e. Postgres rechecks it after the scan.

use dbisam_sql::{ColExpr, CmpOp, Pred, Scalar};
use supabase_wrappers::prelude::{Cell, Qual, Value};

/// Map a Postgres qual list (implicitly AND-ed) into predicates.
pub fn to_preds(quals: &[Qual]) -> Vec<Pred> {
    quals.iter().map(qual_to_pred).collect()
}

fn qual_to_pred(q: &Qual) -> Pred {
    // Parameterised quals are evaluated at execution time, not known here.
    if q.param.is_some() {
        return Pred::Unsupported;
    }
    let field = q.field.clone();

    // IS NULL / IS NOT NULL — Wrappers encodes these as operator "is"/"is not"
    // with the sentinel value Cell::String("null"). DBISAM's IS [NOT] NULL is
    // ANSI-safe (doc 04 quirk #4), so it pushes directly.
    if q.operator == "is" || q.operator == "is not" {
        if let Value::Cell(Cell::String(s)) = &q.value {
            if s == "null" {
                return Pred::IsNull { col: field, negated: q.operator == "is not" };
            }
        }
        // "is"/"is not" with a non-null value is a BoolTest (col IS TRUE/FALSE)
        // — 3-valued logic; leave it to Postgres.
        return Pred::Unsupported;
    }

    match &q.value {
        // `col = ANY(array)` → IN ; `col <> ALL(array)` → NOT IN.
        Value::Array(cells) => {
            let Some(values) = cells.iter().map(cell_to_scalar).collect::<Option<Vec<_>>>() else {
                return Pred::Unsupported;
            };
            match (q.operator.as_str(), q.use_or) {
                ("=", true) => Pred::In { col: ColExpr::Col(field), values, negated: false },
                ("<>", false) => Pred::In { col: ColExpr::Col(field), values, negated: true },
                _ => Pred::Unsupported,
            }
        }
        Value::Cell(c) => {
            // Prefix LIKE only (quirk #3).
            if q.operator == "~~" {
                if let Cell::String(s) = c {
                    if let Some(prefix) = like_prefix(s) {
                        return Pred::LikePrefix { col: field, prefix };
                    }
                }
                return Pred::Unsupported;
            }
            match (cmp_op(&q.operator), cell_to_scalar(c)) {
                (Some(op), Some(value)) => Pred::Compare { left: ColExpr::Col(field), op, value },
                _ => Pred::Unsupported,
            }
        }
    }
}

fn cmp_op(op: &str) -> Option<CmpOp> {
    Some(match op {
        "=" => CmpOp::Eq,
        "<>" => CmpOp::Ne,
        "<" => CmpOp::Lt,
        ">" => CmpOp::Gt,
        "<=" => CmpOp::Le,
        ">=" => CmpOp::Ge,
        _ => return None,
    })
}

/// Convert a Wrappers cell to a renderable scalar, or `None` for types we don't
/// yet push (dates/times need their DBISAM `#…#` literal pinned vs Dibdog —
/// see `dbisam_sql::Scalar`).
fn cell_to_scalar(c: &Cell) -> Option<Scalar> {
    Some(match c {
        Cell::String(s) => Scalar::Text(s.clone()),
        Cell::Bool(b) => Scalar::Bool(*b),
        Cell::I8(n) => Scalar::Int(*n as i64),
        Cell::I16(n) => Scalar::Int(*n as i64),
        Cell::I32(n) => Scalar::Int(*n as i64),
        Cell::I64(n) => Scalar::Int(*n),
        Cell::F32(n) => Scalar::Float(*n as f64),
        Cell::F64(n) => Scalar::Float(*n),
        Cell::Date(dt) => Scalar::Date {
            y: dt.year(),
            m: dt.month() as u32,
            d: dt.day() as u32,
        },
        Cell::Timestamp(ts) => {
            // Only push whole-second timestamps so the 'YYYY-MM-DD HH:MM:SS'
            // literal is exact, not a truncation of a fractional second.
            let sec = ts.second();
            if sec.fract() != 0.0 {
                return None;
            }
            Scalar::Timestamp {
                y: ts.year(),
                mo: ts.month() as u32,
                d: ts.day() as u32,
                h: ts.hour() as u32,
                mi: ts.minute() as u32,
                s: sec as u32,
            }
        }
        _ => return None,
    })
}

/// If `pat` is a pure prefix pattern (`abc%` with no other `%`/`_` wildcards),
/// return the literal prefix (`abc`); otherwise `None`.
///
/// `\` is also rejected: it's Postgres's LIKE escape character, so `abc\%`
/// means the literal string `abc%` — pushing it as `LIKE 'abc\%'` would make
/// DBISAM match a literal backslash instead and silently drop the right rows.
fn like_prefix(pat: &str) -> Option<String> {
    let prefix = pat.strip_suffix('%')?;
    if prefix.contains('%') || prefix.contains('_') || prefix.contains('\\') {
        return None;
    }
    Some(prefix.to_string())
}

#[cfg(test)]
mod tests {
    use super::like_prefix;

    #[test]
    fn like_prefix_accepts_only_trailing_wildcard() {
        assert_eq!(like_prefix("abc%"), Some("abc".to_string()));
        assert_eq!(like_prefix("%abc"), None); // leading
        assert_eq!(like_prefix("a%c%"), None); // internal
        assert_eq!(like_prefix("a_c%"), None); // underscore wildcard
        assert_eq!(like_prefix("abc"), None); // not a LIKE prefix at all
    }

    #[test]
    fn like_prefix_rejects_escape_sequences() {
        assert_eq!(like_prefix("abc\\%"), None); // escaped % — means literal "abc%"
        assert_eq!(like_prefix("abc\\%x%"), None); // escaped % mid-pattern
        assert_eq!(like_prefix("a\\\\c%"), None); // escaped backslash — literal "a\c"
    }
}
