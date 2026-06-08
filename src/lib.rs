/*!
prompt-eval-rubric: score LLM outputs against named 0.0–1.0 rubrics.

Validators reject; rubrics rank. Use this when you want to know *how good*
an output is on each axis, not just whether it passes a binary gate.

```rust
use prompt_eval_rubric::{Rubric, RubricSet};
use serde_json::json;

let length_ok = Rubric::new("length", |out, _ctx| {
    let n = out.len();
    if n >= 10 && n <= 200 { 1.0 } else { 0.0 }
});

let has_json = Rubric::new("has_json", |out, _ctx| {
    if serde_json::from_str::<serde_json::Value>(out).is_ok() { 1.0 } else { 0.0 }
});

let set = RubricSet::new(vec![(length_ok, 0.4), (has_json, 0.6)]).unwrap();
let report = set.evaluate(r#"{"key": "value"}"#, None);

assert!(report.overall > 0.5);
println!("overall: {}", report.overall);
for s in &report.scores {
    println!("{}: {:.2} {:?}", s.name, s.value, s.reason);
}
```
*/

use serde_json::Value;
use std::sync::Arc;

// ---- Score / Report -------------------------------------------------------

/// One rubric's score for one output.
#[derive(Debug, Clone, PartialEq)]
pub struct Score {
    pub name: String,
    /// Clipped to [0.0, 1.0].
    pub value: f64,
    /// Optional human-readable reason or debug note.
    pub reason: Option<String>,
}

/// Aggregate result from `RubricSet::evaluate`.
#[derive(Debug, Clone)]
pub struct Report {
    pub scores: Vec<Score>,
    /// Weighted average of all scores in [0.0, 1.0].
    pub overall: f64,
}

impl Report {
    /// Look up a score by rubric name.
    pub fn by_name(&self, name: &str) -> Option<&Score> {
        self.scores.iter().find(|s| s.name == name)
    }
}

// ---- Rubric ---------------------------------------------------------------

type ScoreFn = Arc<dyn Fn(&str, Option<&Value>) -> f64 + Send + Sync>;
type ScoreWithReasonFn = Arc<dyn Fn(&str, Option<&Value>) -> (f64, Option<String>) + Send + Sync>;

enum Inner {
    Simple(ScoreFn),
    WithReason(ScoreWithReasonFn),
}

/// One scoring axis, wrapping a callable `(output, context) -> score`.
pub struct Rubric {
    pub name: String,
    inner: Inner,
}

impl Rubric {
    /// Create from a simple `f(output, context) -> f64` scorer.
    pub fn new(
        name: impl Into<String>,
        score: impl Fn(&str, Option<&Value>) -> f64 + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            inner: Inner::Simple(Arc::new(score)),
        }
    }

    /// Create from a `f(output, context) -> (f64, Option<String>)` scorer
    /// that also returns a reason string.
    pub fn with_reason(
        name: impl Into<String>,
        score: impl Fn(&str, Option<&Value>) -> (f64, Option<String>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            inner: Inner::WithReason(Arc::new(score)),
        }
    }

    /// Evaluate this rubric against `output`. Exceptions produce 0.0.
    pub fn evaluate(&self, output: &str, context: Option<&Value>) -> Score {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &self.inner {
            Inner::Simple(f) => {
                let v = f(output, context);
                (v, None)
            }
            Inner::WithReason(f) => f(output, context),
        }));
        match result {
            Ok((v, r)) => Score {
                name: self.name.clone(),
                value: clip01(v),
                reason: r,
            },
            Err(e) => {
                let msg = if let Some(s) = e.downcast_ref::<&str>() {
                    format!("panic: {s}")
                } else if let Some(s) = e.downcast_ref::<String>() {
                    format!("panic: {s}")
                } else {
                    "panic in rubric scorer".to_owned()
                };
                Score {
                    name: self.name.clone(),
                    value: 0.0,
                    reason: Some(msg),
                }
            }
        }
    }
}

impl std::fmt::Debug for Rubric {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rubric({})", self.name)
    }
}

impl std::fmt::Debug for RubricSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RubricSet({:?})", self.rubric_names())
    }
}

// ---- RubricSet ------------------------------------------------------------

/// Aggregate multiple rubrics with optional weights.
///
/// Pass bare `(Rubric, weight)` tuples; weights need not sum to 1.0.
/// They are normalized during `evaluate`.
pub struct RubricSet {
    entries: Vec<(Rubric, f64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RubricSetError {
    Empty,
    DuplicateName(String),
    NegativeWeight(String),
}

impl std::fmt::Display for RubricSetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RubricSetError::Empty => write!(f, "RubricSet requires at least one rubric"),
            RubricSetError::DuplicateName(n) => write!(f, "duplicate rubric name: {n}"),
            RubricSetError::NegativeWeight(n) => write!(f, "negative weight for rubric: {n}"),
        }
    }
}

impl std::error::Error for RubricSetError {}

impl RubricSet {
    pub fn new(entries: Vec<(Rubric, f64)>) -> Result<Self, RubricSetError> {
        if entries.is_empty() {
            return Err(RubricSetError::Empty);
        }
        let mut seen = std::collections::HashSet::new();
        for (r, w) in &entries {
            if *w < 0.0 {
                return Err(RubricSetError::NegativeWeight(r.name.clone()));
            }
            if !seen.insert(r.name.clone()) {
                return Err(RubricSetError::DuplicateName(r.name.clone()));
            }
        }
        Ok(Self { entries })
    }

    /// Create with equal weights (all 1.0).
    pub fn uniform(rubrics: Vec<Rubric>) -> Result<Self, RubricSetError> {
        let entries = rubrics.into_iter().map(|r| (r, 1.0)).collect();
        Self::new(entries)
    }

    pub fn rubric_names(&self) -> Vec<&str> {
        self.entries.iter().map(|(r, _)| r.name.as_str()).collect()
    }

    /// Evaluate all rubrics and compute a weighted overall score.
    pub fn evaluate(&self, output: &str, context: Option<&Value>) -> Report {
        let scores: Vec<Score> = self
            .entries
            .iter()
            .map(|(r, _)| r.evaluate(output, context))
            .collect();

        let total_weight: f64 = self.entries.iter().map(|(_, w)| w).sum();
        let overall = if total_weight == 0.0 || total_weight.is_nan() {
            0.0
        } else {
            self.entries
                .iter()
                .zip(&scores)
                .map(|((_, w), s)| s.value * w)
                .sum::<f64>()
                / total_weight
        };

        Report {
            scores,
            overall: clip01(overall),
        }
    }
}

fn clip01(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

// ---- tests ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rubric(name: &'static str, v: f64) -> Rubric {
        Rubric::new(name, move |_, _| v)
    }

    #[test]
    fn single_rubric_evaluate() {
        let r = Rubric::new("length", |out, _| if out.len() >= 5 { 1.0 } else { 0.0 });
        let s = r.evaluate("hello world", None);
        assert_eq!(s.name, "length");
        assert_eq!(s.value, 1.0);
        assert!(s.reason.is_none());
    }

    #[test]
    fn single_rubric_with_reason() {
        let r = Rubric::with_reason("check", |out, _| {
            let n = out.len();
            if n > 5 {
                (1.0, Some(format!("len={n} ok")))
            } else {
                (0.0, Some("too short".to_owned()))
            }
        });
        let s = r.evaluate("hello world", None);
        assert_eq!(s.value, 1.0);
        assert!(s.reason.unwrap().contains("ok"));

        let s2 = r.evaluate("hi", None);
        assert_eq!(s2.value, 0.0);
        assert_eq!(s2.reason.as_deref(), Some("too short"));
    }

    #[test]
    fn clips_value_below_zero() {
        let r = Rubric::new("neg", |_, _| -5.0);
        assert_eq!(r.evaluate("x", None).value, 0.0);
    }

    #[test]
    fn clips_value_above_one() {
        let r = Rubric::new("high", |_, _| 99.0);
        assert_eq!(r.evaluate("x", None).value, 1.0);
    }

    #[test]
    fn panicking_rubric_scores_zero() {
        let r = Rubric::new("panic_rubric", |_, _| panic!("oops"));
        let s = r.evaluate("anything", None);
        assert_eq!(s.value, 0.0);
        assert!(s.reason.as_deref().unwrap_or("").contains("panic"));
    }

    #[test]
    fn rubric_set_uniform_overall() {
        let set = RubricSet::uniform(vec![
            rubric("r1", 1.0),
            rubric("r2", 0.5),
            rubric("r3", 0.0),
        ])
        .unwrap();
        let r = set.evaluate("x", None);
        // (1.0 + 0.5 + 0.0) / 3 = 0.5
        assert!((r.overall - 0.5).abs() < 1e-9);
    }

    #[test]
    fn rubric_set_weighted() {
        let a = Rubric::new("a", |_, _| 1.0);
        let b = Rubric::new("b", |_, _| 0.0);
        let set = RubricSet::new(vec![(a, 0.8), (b, 0.2)]).unwrap();
        let r = set.evaluate("x", None);
        // (1.0*0.8 + 0.0*0.2) / 1.0 = 0.8
        assert!((r.overall - 0.8).abs() < 1e-9);
    }

    #[test]
    fn report_by_name() {
        let set = RubricSet::uniform(vec![
            Rubric::new("alpha", |_, _| 0.9),
            Rubric::new("beta", |_, _| 0.3),
        ])
        .unwrap();
        let r = set.evaluate("test", None);
        let alpha = r.by_name("alpha").unwrap();
        assert_eq!(alpha.value, 0.9);
        let beta = r.by_name("beta").unwrap();
        assert_eq!(beta.value, 0.3);
        assert!(r.by_name("gamma").is_none());
    }

    #[test]
    fn empty_rubric_set_errors() {
        let err = RubricSet::new(vec![]).unwrap_err();
        assert_eq!(err, RubricSetError::Empty);
    }

    #[test]
    fn duplicate_name_errors() {
        let a = Rubric::new("dup", |_, _| 1.0);
        let b = Rubric::new("dup", |_, _| 0.5);
        let err = RubricSet::new(vec![(a, 1.0), (b, 1.0)]).unwrap_err();
        assert!(matches!(err, RubricSetError::DuplicateName(_)));
    }

    #[test]
    fn negative_weight_errors() {
        let a = Rubric::new("a", |_, _| 1.0);
        let err = RubricSet::new(vec![(a, -1.0)]).unwrap_err();
        assert!(matches!(err, RubricSetError::NegativeWeight(_)));
    }

    #[test]
    fn rubric_receives_context() {
        let r = Rubric::new("ctx_check", |_out, ctx| {
            if ctx.and_then(|c| c.as_str()) == Some("expected") {
                1.0
            } else {
                0.0
            }
        });
        let s = r.evaluate("anything", Some(&json!("expected")));
        assert_eq!(s.value, 1.0);
        let s2 = r.evaluate("anything", Some(&json!("wrong")));
        assert_eq!(s2.value, 0.0);
    }

    #[test]
    fn rubric_no_context() {
        let r = Rubric::new("no_ctx", |out, ctx| {
            assert!(ctx.is_none());
            out.len() as f64 / 100.0
        });
        let s = r.evaluate("hi", None);
        assert_eq!(s.value, 0.02);
    }

    #[test]
    fn zero_weight_in_set_excluded_from_overall() {
        let a = Rubric::new("a", |_, _| 0.0);
        let b = Rubric::new("b", |_, _| 1.0);
        let set = RubricSet::new(vec![(a, 0.0), (b, 1.0)]).unwrap();
        let r = set.evaluate("x", None);
        assert!((r.overall - 1.0).abs() < 1e-9);
    }

    #[test]
    fn scores_count_matches_rubric_count() {
        let set = RubricSet::uniform(vec![
            Rubric::new("r1", |_, _| 1.0),
            Rubric::new("r2", |_, _| 0.5),
            Rubric::new("r3", |_, _| 0.0),
        ])
        .unwrap();
        let r = set.evaluate("x", None);
        assert_eq!(r.scores.len(), 3);
    }

    #[test]
    fn rubric_names_accessor() {
        let set = RubricSet::uniform(vec![
            Rubric::new("alpha", |_, _| 1.0),
            Rubric::new("beta", |_, _| 0.0),
        ])
        .unwrap();
        let names = set.rubric_names();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn nan_output_from_scorer_clips_to_zero() {
        let r = Rubric::new("nan_r", |_, _| f64::NAN);
        assert_eq!(r.evaluate("x", None).value, 0.0);
    }
}
