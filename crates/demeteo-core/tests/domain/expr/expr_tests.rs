//! Exhaustive coverage for the expression mini-evaluator (task P1.5):
//! the full grammar accepted, everything outside it rejected.

use super::*;

fn ctx(
    entries: Vec<(&'static str, &'static str, ExprValue)>,
) -> impl Fn(&str, &str) -> Option<ExprValue> {
    move |node: &str, output: &str| {
        entries
            .iter()
            .find(|(n, o, _)| *n == node && *o == output)
            .map(|(_, _, v)| v.clone())
    }
}

// ---------- accepted grammar ----------

#[test]
fn prd_example_evaluates() {
    let resolve = ctx(vec![(
        "critic",
        "verdict",
        ExprValue::Str("PASS_WITH_NOTES".into()),
    )]);
    assert!(evaluate("${{ nodes.critic.outputs.verdict != 'FAIL' }}", &resolve).unwrap());

    let resolve = ctx(vec![("critic", "verdict", ExprValue::Str("FAIL".into()))]);
    assert!(!evaluate("${{ nodes.critic.outputs.verdict != 'FAIL' }}", &resolve).unwrap());
}

#[test]
fn all_comparison_operators() {
    let resolve = ctx(vec![("n", "x", ExprValue::Num(3.0))]);
    for (expr, expected) in [
        ("${{ nodes.n.outputs.x == 3 }}", true),
        ("${{ nodes.n.outputs.x != 3 }}", false),
        ("${{ nodes.n.outputs.x < 4 }}", true),
        ("${{ nodes.n.outputs.x <= 3 }}", true),
        ("${{ nodes.n.outputs.x > 3 }}", false),
        ("${{ nodes.n.outputs.x >= 3.0 }}", true),
        ("${{ nodes.n.outputs.x > -1.5 }}", true),
    ] {
        assert_eq!(evaluate(expr, &resolve).unwrap(), expected, "{expr}");
    }
}

#[test]
fn literal_only_comparisons_and_bools() {
    let none = ctx(vec![]);
    assert!(evaluate("${{ 'a' == 'a' }}", &none).unwrap());
    assert!(evaluate("${{ 1.5 < 2 }}", &none).unwrap());
    assert!(evaluate("${{ true == true }}", &none).unwrap());
    assert!(!evaluate("${{ true == false }}", &none).unwrap());
    assert!(evaluate("${{ true }}", &none).unwrap());
    assert!(!evaluate("${{ false }}", &none).unwrap());
}

#[test]
fn bare_boolean_output_is_the_guard_result() {
    let resolve = ctx(vec![("gate", "approved", ExprValue::Bool(true))]);
    assert!(evaluate("${{ nodes.gate.outputs.approved }}", &resolve).unwrap());
}

#[test]
fn idents_accept_hyphens_and_underscores() {
    let resolve = ctx(vec![(
        "s-critic_2",
        "final-verdict",
        ExprValue::Str("PASS".into()),
    )]);
    assert!(evaluate(
        "${{ nodes.s-critic_2.outputs.final-verdict == 'PASS' }}",
        &resolve
    )
    .unwrap());
}

#[test]
fn whitespace_is_flexible() {
    let resolve = ctx(vec![("n", "x", ExprValue::Num(1.0))]);
    for expr in [
        "${{nodes.n.outputs.x==1}}",
        "  ${{   nodes.n.outputs.x   ==   1   }}  ",
        "${{ nodes.n.outputs.x ==1 }}",
    ] {
        assert!(evaluate(expr, &resolve).unwrap(), "{expr}");
    }
}

#[test]
fn empty_string_literal_is_legal() {
    let resolve = ctx(vec![("n", "s", ExprValue::Str(String::new()))]);
    assert!(evaluate("${{ nodes.n.outputs.s == '' }}", &resolve).unwrap());
}

#[test]
fn referenced_paths_are_reported_for_editor_validation() {
    let expr =
        parse("${{ nodes.critic.outputs.verdict != nodes.validate.outputs.verdict }}").unwrap();
    assert_eq!(
        expr.referenced_paths(),
        vec![("critic", "verdict"), ("validate", "verdict")]
    );
}

// ---------- rejections: everything outside the grammar ----------

#[test]
fn rejects_everything_outside_the_grammar() {
    let cases: &[&str] = &[
        // wrapper problems
        "nodes.a.outputs.x == 1",     // no ${{ }}
        "${{ nodes.a.outputs.x == 1", // unterminated wrapper
        "prefix ${{ true }}",         // text outside
        "${{ true }} suffix",         // text outside
        "${{ ${{ true }} }}",         // nested
        "",                           // empty
        "${{ }}",                     // empty body
        // connectives / operators we deliberately don't have
        "${{ true && false }}",
        "${{ true || false }}",
        "${{ !true }}",
        "${{ (true) }}",
        "${{ 1 + 1 == 2 }}",
        "${{ nodes.a.outputs.x = 1 }}",   // single =
        "${{ nodes.a.outputs.x === 1 }}", // trailing junk
        // functions / indexing / other roots
        "${{ contains(nodes.a.outputs.x, 'y') }}",
        "${{ nodes['a'].outputs.x == 1 }}",
        "${{ steps.a.outputs.x == 1 }}",
        "${{ env.HOME == 'x' }}",
        "${{ nodes.a.results.x == 1 }}", // wrong section
        "${{ nodes.a.outputs == 1 }}",   // path too short
        "${{ nodes.a }}",
        "${{ nodes }}",
        // literal problems
        "${{ 'unterminated }}",
        "${{ \"double\" == 'x' }}", // double quotes
        "${{ 1. == 1 }}",           // bare decimal point
        "${{ - == 1 }}",            // dangling minus
        "${{ 07x == 1 }}",          // junk after digits
        // structure problems
        "${{ == 1 }}",        // missing lhs
        "${{ 1 == }}",        // missing rhs
        "${{ 1 == 2 == 3 }}", // chained comparison
        "${{ maybe }}",       // unknown keyword
    ];
    for expr in cases {
        let result = parse(expr);
        assert!(
            matches!(result, Err(ExprError::Syntax(_))),
            "should reject {expr:?}, got {result:?}"
        );
    }
}

#[test]
fn number_junk_is_rejected_at_parse() {
    // digits followed by identifier chars parse the number then trip on
    // trailing input — either way, rejected.
    assert!(parse("${{ 12abc == 1 }}").is_err());
}

// ---------- type discipline ----------

#[test]
fn cross_type_equality_is_an_error_not_false() {
    let none = ctx(vec![]);
    for expr in [
        "${{ 'a' == 1 }}",
        "${{ true == 'true' }}",
        "${{ 1 == true }}",
    ] {
        assert!(
            matches!(evaluate(expr, &none), Err(ExprError::TypeMismatch(_))),
            "{expr}"
        );
    }
}

#[test]
fn ordering_is_numeric_only() {
    let none = ctx(vec![]);
    for expr in ["${{ 'a' < 'b' }}", "${{ true < false }}", "${{ 'a' <= 1 }}"] {
        assert!(
            matches!(evaluate(expr, &none), Err(ExprError::TypeMismatch(_))),
            "{expr}"
        );
    }
}

#[test]
fn bare_non_boolean_term_is_a_type_error() {
    let resolve = ctx(vec![("n", "s", ExprValue::Str("yes".into()))]);
    assert!(matches!(
        evaluate("${{ nodes.n.outputs.s }}", &resolve),
        Err(ExprError::TypeMismatch(_))
    ));
    assert!(matches!(
        evaluate("${{ 'yes' }}", &resolve),
        Err(ExprError::TypeMismatch(_))
    ));
    assert!(matches!(
        evaluate("${{ 3 }}", &resolve),
        Err(ExprError::TypeMismatch(_))
    ));
}

#[test]
fn unknown_output_is_an_error_not_false() {
    let none = ctx(vec![]);
    let err = evaluate("${{ nodes.ghost.outputs.x == 1 }}", &none).unwrap_err();
    assert_eq!(
        err,
        ExprError::UnknownOutput {
            node: "ghost".into(),
            output: "x".into()
        }
    );
    assert!(err.to_string().contains("ghost"), "readable: {err}");
}
