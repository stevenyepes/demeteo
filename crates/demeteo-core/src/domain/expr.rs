//! Sandboxed expression mini-evaluator for edge guards and input
//! bindings (task P1.5, PRD §5.1 "Expressions").
//!
//! # Grammar — all of it
//!
//! ```text
//! guard      := ws "${{" ws expr ws "}}" ws          (exactly one, nothing outside)
//! expr       := term ( ws cmp ws term )?
//! cmp        := "==" | "!=" | "<" | "<=" | ">" | ">="
//! term       := path | literal
//! path       := "nodes" "." ident "." "outputs" "." ident
//! literal    := string | number | "true" | "false"
//! string     := "'" [^']* "'"                        (single quotes, no escapes)
//! number     := "-"? digits ( "." digits )?
//! ident      := [A-Za-z0-9_-]+
//! ```
//!
//! That is the whole language, deliberately (PRD §10 scope-creep risk):
//! no boolean connectives, no arithmetic, no parentheses, no function
//! calls, no indexing, no string escapes, no context roots other than
//! `nodes.<id>.outputs.<name>`. Anything outside the grammar is a
//! [`ExprError::Syntax`] — growing the language requires a new decision
//! record, not a parser patch.
//!
//! # Semantics
//!
//! - A bare term must resolve to a boolean — that boolean is the guard
//!   result. A bare string/number is a type error, not truthiness.
//! - `==` / `!=` compare same-typed values only (string↔string,
//!   number↔number, bool↔bool); comparing across types is a type error,
//!   not `false` — a guard that can never be true is authored wrong.
//! - `<` `<=` `>` `>=` are numeric only.
//! - An unresolvable path (unknown node or output) is an error, not a
//!   silent `false`; the scheduler surfaces it as the skip/fail reason.
//!
//! [`parse`] exists separately from [`evaluate`] so the editor can
//! validate a guard without run state (the `expr_validate` seam, P4.3).

use std::fmt;

/// A value a node output can carry into an expression.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprValue {
    Str(String),
    Num(f64),
    Bool(bool),
}

impl fmt::Display for ExprValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExprValue::Str(s) => write!(f, "'{s}'"),
            ExprValue::Num(n) => write!(f, "{n}"),
            ExprValue::Bool(b) => write!(f, "{b}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprError {
    /// The text is outside the grammar. Carries a human-readable reason.
    Syntax(String),
    /// A `nodes.<id>.outputs.<name>` path that the resolver has no value
    /// for at evaluation time.
    UnknownOutput { node: String, output: String },
    /// Grammatically valid but type-incoherent (e.g. `'a' < 3`).
    TypeMismatch(String),
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExprError::Syntax(msg) => write!(f, "invalid expression: {msg}"),
            ExprError::UnknownOutput { node, output } => {
                write!(f, "no output '{output}' recorded for node '{node}'")
            }
            ExprError::TypeMismatch(msg) => write!(f, "type mismatch: {msg}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Term {
    /// `nodes.<node>.outputs.<output>`
    Path {
        node: String,
        output: String,
    },
    Lit(ExprValue),
}

/// A parsed guard: one term, optionally compared against another.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub lhs: Term,
    pub cmp: Option<(CmpOp, Term)>,
}

impl Expr {
    /// The node/output paths this expression reads — the editor uses
    /// this to validate references against the graph (P4.3).
    pub fn referenced_paths(&self) -> Vec<(&str, &str)> {
        let mut paths = Vec::new();
        for term in std::iter::once(&self.lhs).chain(self.cmp.iter().map(|(_, t)| t)) {
            if let Term::Path { node, output } = term {
                paths.push((node.as_str(), output.as_str()));
            }
        }
        paths
    }
}

/// Parse a guard string (`${{ … }}` wrapper included). Rejects anything
/// outside the module-doc grammar.
pub fn parse(input: &str) -> Result<Expr, ExprError> {
    let trimmed = input.trim();
    let inner = trimmed
        .strip_prefix("${{")
        .and_then(|rest| rest.strip_suffix("}}"))
        .ok_or_else(|| {
            ExprError::Syntax("guard must be exactly one '${{ … }}' interpolation".into())
        })?;
    if inner.contains("${{") {
        return Err(ExprError::Syntax("nested '${{' is not allowed".into()));
    }

    let mut p = Parser {
        chars: inner.chars().collect(),
        pos: 0,
    };
    p.skip_ws();
    let lhs = p.term()?;
    p.skip_ws();
    let cmp = if p.done() {
        None
    } else {
        let op = p.cmp_op()?;
        p.skip_ws();
        let rhs = p.term()?;
        p.skip_ws();
        if !p.done() {
            return Err(ExprError::Syntax(format!(
                "unexpected trailing input at '{}'",
                p.rest()
            )));
        }
        Some((op, rhs))
    };
    Ok(Expr { lhs, cmp })
}

/// Parse and evaluate a guard. `resolve` supplies node outputs;
/// returning `None` yields [`ExprError::UnknownOutput`].
pub fn evaluate(
    input: &str,
    resolve: &dyn Fn(&str, &str) -> Option<ExprValue>,
) -> Result<bool, ExprError> {
    let expr = parse(input)?;
    let lhs = resolve_term(&expr.lhs, resolve)?;
    match &expr.cmp {
        None => match lhs {
            ExprValue::Bool(b) => Ok(b),
            other => Err(ExprError::TypeMismatch(format!(
                "bare term must be a boolean, got {other}"
            ))),
        },
        Some((op, rhs_term)) => {
            let rhs = resolve_term(rhs_term, resolve)?;
            compare(*op, &lhs, &rhs)
        }
    }
}

fn resolve_term(
    term: &Term,
    resolve: &dyn Fn(&str, &str) -> Option<ExprValue>,
) -> Result<ExprValue, ExprError> {
    match term {
        Term::Lit(v) => Ok(v.clone()),
        Term::Path { node, output } => {
            resolve(node, output).ok_or_else(|| ExprError::UnknownOutput {
                node: node.clone(),
                output: output.clone(),
            })
        }
    }
}

fn compare(op: CmpOp, lhs: &ExprValue, rhs: &ExprValue) -> Result<bool, ExprError> {
    use ExprValue::*;
    match op {
        CmpOp::Eq | CmpOp::Ne => {
            let eq = match (lhs, rhs) {
                (Str(a), Str(b)) => a == b,
                (Num(a), Num(b)) => a == b,
                (Bool(a), Bool(b)) => a == b,
                _ => {
                    return Err(ExprError::TypeMismatch(format!(
                        "cannot compare {lhs} with {rhs}"
                    )))
                }
            };
            Ok(if op == CmpOp::Eq { eq } else { !eq })
        }
        CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge => match (lhs, rhs) {
            (Num(a), Num(b)) => Ok(match op {
                CmpOp::Lt => a < b,
                CmpOp::Le => a <= b,
                CmpOp::Gt => a > b,
                CmpOp::Ge => a >= b,
                _ => unreachable!(),
            }),
            _ => Err(ExprError::TypeMismatch(format!(
                "ordering compares numbers only, got {lhs} and {rhs}"
            ))),
        },
    }
}

// ---------- the tiny hand parser ----------

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn done(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn rest(&self) -> String {
        self.chars[self.pos.min(self.chars.len())..]
            .iter()
            .collect()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn term(&mut self) -> Result<Term, ExprError> {
        match self.peek() {
            Some('\'') => self.string_lit(),
            Some(c) if c.is_ascii_digit() || c == '-' => self.number_lit(),
            Some(c) if c.is_ascii_alphanumeric() || c == '_' => {
                let word = self.ident()?;
                match word.as_str() {
                    "true" => Ok(Term::Lit(ExprValue::Bool(true))),
                    "false" => Ok(Term::Lit(ExprValue::Bool(false))),
                    "nodes" => self.path_tail(),
                    other => Err(ExprError::Syntax(format!(
                        "unknown identifier '{other}' — only 'nodes.<id>.outputs.<name>' \
                         paths, quoted strings, numbers, true, and false are allowed"
                    ))),
                }
            }
            Some(c) => Err(ExprError::Syntax(format!("unexpected character '{c}'"))),
            None => Err(ExprError::Syntax("expected a term, found end".into())),
        }
    }

    /// After the `nodes` keyword: `.ident.outputs.ident`.
    fn path_tail(&mut self) -> Result<Term, ExprError> {
        self.expect('.')?;
        let node = self.ident()?;
        self.expect('.')?;
        let section = self.ident()?;
        if section != "outputs" {
            return Err(ExprError::Syntax(format!(
                "expected 'outputs' after 'nodes.{node}.', got '{section}'"
            )));
        }
        self.expect('.')?;
        let output = self.ident()?;
        Ok(Term::Path { node, output })
    }

    fn expect(&mut self, c: char) -> Result<(), ExprError> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(ExprError::Syntax(format!(
                "expected '{c}' at '{}'",
                self.rest()
            )))
        }
    }

    fn ident(&mut self) -> Result<String, ExprError> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(ExprError::Syntax(format!(
                "expected an identifier at '{}'",
                self.rest()
            )));
        }
        Ok(self.chars[start..self.pos].iter().collect())
    }

    fn string_lit(&mut self) -> Result<Term, ExprError> {
        self.expect('\'')?;
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c != '\'') {
            self.pos += 1;
        }
        if self.done() {
            return Err(ExprError::Syntax("unterminated string literal".into()));
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        self.pos += 1; // closing quote
        Ok(Term::Lit(ExprValue::Str(s)))
    }

    fn number_lit(&mut self) -> Result<Term, ExprError> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        let digits_start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == digits_start {
            return Err(ExprError::Syntax(format!(
                "expected digits at '{}'",
                self.rest()
            )));
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            let frac_start = self.pos;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
            if self.pos == frac_start {
                return Err(ExprError::Syntax(
                    "expected digits after decimal point".into(),
                ));
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        let n: f64 = text
            .parse()
            .map_err(|_| ExprError::Syntax(format!("invalid number '{text}'")))?;
        Ok(Term::Lit(ExprValue::Num(n)))
    }

    fn cmp_op(&mut self) -> Result<CmpOp, ExprError> {
        let (op, len) = match (self.peek(), self.chars.get(self.pos + 1).copied()) {
            (Some('='), Some('=')) => (CmpOp::Eq, 2),
            (Some('!'), Some('=')) => (CmpOp::Ne, 2),
            (Some('<'), Some('=')) => (CmpOp::Le, 2),
            (Some('>'), Some('=')) => (CmpOp::Ge, 2),
            (Some('<'), _) => (CmpOp::Lt, 1),
            (Some('>'), _) => (CmpOp::Gt, 1),
            _ => {
                return Err(ExprError::Syntax(format!(
                    "expected a comparison operator at '{}'",
                    self.rest()
                )))
            }
        };
        self.pos += len;
        Ok(op)
    }
}

#[cfg(test)]
#[path = "../../tests/domain/expr/expr_tests.rs"]
mod expr_tests;
