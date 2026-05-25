# prompt-eval-rubric

Score LLM outputs against named 0.0–1.0 rubrics. Rubrics rank; validators reject.

## Usage

```rust
use prompt_eval_rubric::{Rubric, RubricSet};

let length = Rubric::new("length", |out, _| {
    let n = out.len();
    if n >= 50 && n <= 500 { 1.0 } else { 0.0 }
});
let is_json = Rubric::new("is_json", |out, _| {
    if serde_json::from_str::<serde_json::Value>(out).is_ok() { 1.0 } else { 0.0 }
});

let set = RubricSet::new(vec![(length, 0.3), (is_json, 0.7)]).unwrap();
let report = set.evaluate(r#"{"answer": "Paris"}"#, None);
println!("overall: {}", report.overall);
```

## License

MIT OR Apache-2.0
