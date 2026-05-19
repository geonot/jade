# alpha_release_demo

Comprehensive Jinn sample project used for alpha release readiness checks.

## What It Exercises

- Multi-file and nested modules (`source/analytics.jn`, `source/models/metric.jn`, `source/workers/ingest.jn`, `source/reports/reporter.jn`)
- Standard library includes (`use std/time`)
- Typed domain model (`Metric` object creation and field access)
- Actor runtime (`Aggregator` with `*loop` flush and method-call messaging)
- Persistent stores:
	- `metrics` (raw event rows)
	- `metric_windows` (windowed aggregate rows)
- Store operations in one workflow:
	- `insert`, `count`, `sum`, `max`, `min`, `exists`, `first`, `set`, `delete`
- Inline tests (`jinn test`)

## Commands

```bash
jinn test
jinn build -o alpha_demo
./alpha_demo
jinn package
```

## Expected Output Markers

`./alpha_demo` prints labeled markers including:

- `metrics_rows`
- `window_rows`
- `sample_count`
- `value_sum`
- `elapsed_ms`
