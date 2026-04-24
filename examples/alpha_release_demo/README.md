# alpha_release_demo

Comprehensive Jade sample project used for alpha release readiness checks.

## What It Exercises

- Multi-file and nested modules (`source/analytics.jade`, `source/models/metric.jade`, `source/workers/ingest.jade`, `source/reports/reporter.jade`)
- Standard library includes (`use std/time`)
- Typed domain model (`Metric` object creation and field access)
- Actor runtime (`Aggregator` with `*loop` flush and method-call messaging)
- Persistent stores:
	- `metrics` (raw event rows)
	- `metric_windows` (windowed aggregate rows)
- Store operations in one workflow:
	- `insert`, `count`, `sum`, `max`, `min`, `exists`, `first`, `set`, `delete`
- Inline tests (`jade test`)

## Commands

```bash
jade test
jade build -o alpha_demo
./alpha_demo
jade package
```

## Expected Output Markers

`./alpha_demo` prints labeled markers including:

- `metrics_rows`
- `window_rows`
- `sample_count`
- `value_sum`
- `elapsed_ms`
