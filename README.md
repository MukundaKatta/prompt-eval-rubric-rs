# prompt-eval-rubric

Score LLM outputs against named `0.0`–`1.0` rubrics. **Rubrics rank; validators reject.**

Use this crate when you want to know *how good* an output is on each axis, not just
whether it passes a binary gate. Each rubric is a small scoring function; a `RubricSet`
runs many of them and aggregates a weighted overall score, isolating panics so one
broken scorer can never take down the rest.

## Why rubrics, not validators?

- A **validator** answers a yes/no question: "is this valid JSON?" — it *rejects*.
- A **rubric** answers a graded question: "how close is this to the ideal length?" — it *ranks*.

Both are useful, but rubrics let you compare two passing outputs and pick the better
one, drive regression dashboards, or feed a reward signal. This crate focuses on the
ranking case while still letting a rubric act as a hard `0.0` / `1.0` gate when you want.

## Features

- **Simple closures as scorers** — `Rubric::new("name", |output, context| -> f64)`.
- **Reasoned scores** — `Rubric::with_reason` returns an explanation alongside the value.
- **Weighted aggregation** — weights need not sum to `1.0`; they are normalized for you.
- **Exception isolation** — a panicking scorer yields `0.0` (with the panic message as
  the reason) instead of aborting the whole evaluation.
- **Score clamping** — every score is clipped into `[0.0, 1.0]`, and `NaN` becomes `0.0`.
- **Optional JSON context** — pass a `serde_json::Value` of reference data (expected
  answer, metadata, ...) to every rubric.

## Install

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
prompt-eval-rubric = "0.1"
serde_json = "1"
```

The public API exposes `serde_json::Value` as the context type, so most users will
also want `serde_json` as a direct dependency.

## Usage

```rust
use prompt_eval_rubric::{Rubric, RubricSet};
use serde_json::json;

// A rubric that rewards outputs in a sensible length band.
let length = Rubric::new("length", |out: &str, _ctx| {
    let n = out.len();
    if (50..=500).contains(&n) { 1.0 } else { 0.0 }
});

// A rubric that checks the output parses as JSON.
let is_json = Rubric::new("is_json", |out: &str, _ctx| {
    if serde_json::from_str::<serde_json::Value>(out).is_ok() { 1.0 } else { 0.0 }
});

// A rubric that uses the context to compare against an expected answer,
// and explains itself via `with_reason`.
let matches_expected = Rubric::with_reason("matches_expected", |out: &str, ctx| {
    let expected = ctx.and_then(|c| c.get("expected")).and_then(|v| v.as_str());
    match expected {
        Some(e) if e == out => (1.0, Some("exact match".to_owned())),
        Some(e) => (0.0, Some(format!("expected {e:?}"))),
        None => (0.0, Some("no expected value in context".to_owned())),
    }
});

// Weights need not sum to 1.0 — they are normalized during `evaluate`.
let set = RubricSet::new(vec![
    (length, 0.2),
    (is_json, 0.3),
    (matches_expected, 0.5),
])
.expect("valid rubric set");

let output = r#"{"answer": "Paris"}"#;
let context = json!({ "expected": r#"{"answer": "Paris"}"# });
let report = set.evaluate(output, Some(&context));

println!("overall: {:.2}", report.overall);
for s in &report.scores {
    println!("  {:<16} {:.2}  {:?}", s.name, s.value, s.reason);
}
```

### Equal weights

When you do not care about weighting, use `RubricSet::uniform` to give every rubric a
weight of `1.0`:

```rust
use prompt_eval_rubric::{Rubric, RubricSet};

let set = RubricSet::uniform(vec![
    Rubric::new("non_empty", |out: &str, _| if out.is_empty() { 0.0 } else { 1.0 }),
    Rubric::new("lowercase", |out: &str, _| {
        if out.chars().all(|c| !c.is_uppercase()) { 1.0 } else { 0.0 }
    }),
])
.unwrap();

let report = set.evaluate("hello", None);
assert_eq!(report.overall, 1.0);
```

## API

| Item | Description |
| --- | --- |
| `Rubric::new(name, fn)` | Build a rubric from `Fn(&str, Option<&Value>) -> f64`. |
| `Rubric::with_reason(name, fn)` | Build a rubric from `Fn(&str, Option<&Value>) -> (f64, Option<String>)`. |
| `Rubric::evaluate(output, ctx)` | Run a single rubric, returning a `Score`. Panics are caught and scored `0.0`. |
| `RubricSet::new(entries)` | Build a set from `(Rubric, weight)` pairs. Errors on empty input, duplicate names, or negative weights. |
| `RubricSet::uniform(rubrics)` | Build a set giving every rubric weight `1.0`. |
| `RubricSet::evaluate(output, ctx)` | Score every rubric and compute a weighted `Report`. |
| `RubricSet::rubric_names()` | The rubric names, in insertion order. |
| `Report::overall` | Weighted average of all scores, clamped to `[0.0, 1.0]`. |
| `Report::scores` | Per-rubric `Score` values (`name`, `value`, optional `reason`). |
| `Report::by_name(name)` | Look up a single `Score` by rubric name. |
| `RubricSetError` | `Empty`, `DuplicateName(String)`, or `NegativeWeight(String)`. |

### Semantics worth knowing

- **Scores are clamped.** A scorer returning `2.0` is recorded as `1.0`; `-1.0` becomes
  `0.0`; `NaN` becomes `0.0`.
- **Weights are normalized.** The overall score is `sum(value_i * weight_i) / sum(weight_i)`.
  If the total weight is `0.0` (or `NaN`), the overall score is `0.0`.
- **Panics are isolated.** If a scorer panics, only that rubric scores `0.0`; the panic
  message is captured in `Score::reason` and the rest of the set still runs.

## Testing

```sh
cargo test
```

Unit tests live alongside the implementation in `src/lib.rs`; integration tests that
drive the public API live in `tests/integration.rs`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual
licensed as above, without any additional terms or conditions.
