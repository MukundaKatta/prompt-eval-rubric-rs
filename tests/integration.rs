//! Integration tests exercising the public API exactly as a downstream
//! crate would: only items reachable through `prompt_eval_rubric::*`.

use prompt_eval_rubric::{Rubric, RubricSet, RubricSetError};
use serde_json::json;

/// The end-to-end example from the README/crate docs should aggregate to a
/// score above 0.5 for a short, valid JSON payload.
#[test]
fn readme_example_end_to_end() {
    let length = Rubric::new("length", |out: &str, _| {
        let n = out.len();
        if (10..=200).contains(&n) {
            1.0
        } else {
            0.0
        }
    });
    let has_json = Rubric::new("has_json", |out: &str, _| {
        if serde_json::from_str::<serde_json::Value>(out).is_ok() {
            1.0
        } else {
            0.0
        }
    });

    let set = RubricSet::new(vec![(length, 0.4), (has_json, 0.6)]).unwrap();
    let report = set.evaluate(r#"{"key": "value"}"#, None);

    assert!(report.overall > 0.5, "overall was {}", report.overall);
    assert_eq!(report.scores.len(), 2);
    assert_eq!(report.by_name("has_json").unwrap().value, 1.0);
    assert_eq!(report.by_name("length").unwrap().value, 1.0);
}

/// Weights need not sum to 1.0; they are normalized by their total.
/// (1.0 * 2 + 0.0 * 6) / (2 + 6) = 0.25
#[test]
fn unnormalized_weights_are_normalized() {
    let good = Rubric::new("good", |_, _| 1.0);
    let bad = Rubric::new("bad", |_, _| 0.0);
    let set = RubricSet::new(vec![(good, 2.0), (bad, 6.0)]).unwrap();
    let report = set.evaluate("anything", None);
    assert!(
        (report.overall - 0.25).abs() < 1e-9,
        "got {}",
        report.overall
    );
}

/// A panicking rubric must not abort the whole set: its score becomes 0.0
/// while the other rubrics still contribute.
#[test]
fn one_panicking_rubric_does_not_poison_the_set() {
    let solid = Rubric::new("solid", |_, _| 1.0);
    let boom = Rubric::new("boom", |_, _| panic!("kaboom"));
    let set = RubricSet::uniform(vec![solid, boom]).unwrap();
    let report = set.evaluate("x", None);

    // (1.0 + 0.0) / 2
    assert!(
        (report.overall - 0.5).abs() < 1e-9,
        "got {}",
        report.overall
    );
    let boom_score = report.by_name("boom").unwrap();
    assert_eq!(boom_score.value, 0.0);
    assert!(boom_score.reason.as_deref().unwrap_or("").contains("panic"));
}

/// Context (the second scorer argument) is threaded through to every rubric.
#[test]
fn context_is_passed_to_rubrics() {
    let expects_paris = Rubric::new("matches_expected", |out: &str, ctx| {
        let expected = ctx.and_then(|c| c.get("expected")).and_then(|v| v.as_str());
        if expected == Some(out) {
            1.0
        } else {
            0.0
        }
    });
    let set = RubricSet::uniform(vec![expects_paris]).unwrap();

    let ctx = json!({ "expected": "Paris" });
    assert_eq!(set.evaluate("Paris", Some(&ctx)).overall, 1.0);
    assert_eq!(set.evaluate("London", Some(&ctx)).overall, 0.0);
}

/// `with_reason` rubrics surface their explanation string in the report.
#[test]
fn with_reason_surfaces_explanation() {
    let r = Rubric::with_reason("verbosity", |out: &str, _| {
        let words = out.split_whitespace().count();
        if words >= 3 {
            (1.0, Some(format!("{words} words")))
        } else {
            (0.0, Some("too terse".to_owned()))
        }
    });
    let set = RubricSet::uniform(vec![r]).unwrap();

    let report = set.evaluate("one two three four", None);
    let score = report.by_name("verbosity").unwrap();
    assert_eq!(score.value, 1.0);
    assert_eq!(score.reason.as_deref(), Some("4 words"));
}

/// Construction errors are reported rather than panicking.
#[test]
fn construction_errors_are_typed() {
    assert_eq!(RubricSet::new(vec![]).unwrap_err(), RubricSetError::Empty);

    let dup_err = RubricSet::new(vec![
        (Rubric::new("same", |_, _| 1.0), 1.0),
        (Rubric::new("same", |_, _| 0.0), 1.0),
    ])
    .unwrap_err();
    assert!(matches!(dup_err, RubricSetError::DuplicateName(name) if name == "same"));

    let neg_err = RubricSet::new(vec![(Rubric::new("a", |_, _| 1.0), -0.5)]).unwrap_err();
    assert!(matches!(neg_err, RubricSetError::NegativeWeight(name) if name == "a"));
}

/// Errors implement `Display` / `std::error::Error` so they can be used with
/// `?` and printed.
#[test]
fn errors_display_human_messages() {
    let err = RubricSet::new(vec![]).unwrap_err();
    assert_eq!(err.to_string(), "RubricSet requires at least one rubric");

    let dup = RubricSetError::DuplicateName("foo".to_owned());
    assert!(dup.to_string().contains("foo"));
}

/// Out-of-range scorer outputs are clamped into [0.0, 1.0] and NaN becomes 0.0.
#[test]
fn scores_are_clamped_into_unit_interval() {
    let high = Rubric::new("high", |_, _| 12.0);
    let low = Rubric::new("low", |_, _| -3.0);
    let nan = Rubric::new("nan", |_, _| f64::NAN);
    let set = RubricSet::uniform(vec![high, low, nan]).unwrap();
    let report = set.evaluate("x", None);

    assert_eq!(report.by_name("high").unwrap().value, 1.0);
    assert_eq!(report.by_name("low").unwrap().value, 0.0);
    assert_eq!(report.by_name("nan").unwrap().value, 0.0);
    // (1.0 + 0.0 + 0.0) / 3
    assert!((report.overall - 1.0 / 3.0).abs() < 1e-9);
}

/// `rubric_names` preserves insertion order.
#[test]
fn rubric_names_preserve_order() {
    let set = RubricSet::uniform(vec![
        Rubric::new("first", |_, _| 1.0),
        Rubric::new("second", |_, _| 1.0),
        Rubric::new("third", |_, _| 1.0),
    ])
    .unwrap();
    assert_eq!(set.rubric_names(), vec!["first", "second", "third"]);
}
