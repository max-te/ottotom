# Ottotom (OpenTelemetry to text OpenMetrics)

[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://app.codspeed.io/max-te/ottotom?utm_source=badge)

A Rust crate for exporting OpenTelemetry metrics in the [OpenMetrics](https://github.com/prometheus/OpenMetrics) text format.
This serves as a protobuf-free alternative to the discontinued `opentelemetry-prometheus` crate.

This implementation tries to follow the [OpenTelemetry-to-OpenMetrics conversion](https://github.com/open-telemetry/opentelemetry-specification/blob/v1.45.0/specification/compatibility/prometheus_and_openmetrics.md) spec,
though it takes some liberties where it contradicts the OpenMetrics spec. Some edge cases and complex metrics setups may not be handled correctly.
See [Tracey spec tracking](#tracey-spec-tracking) for how spec-compliance is tracked.

## Features

- **Conversion** of `opentelemetry-sdk` metric data to OpenMetrics-compliant text.
- **Ready-to-use Exporter** to register in `opentelemetry`, outputs metrics in the OpenMetrics text format.

## Usage

```rust,no_run
use std::time::Duration;
use ottotom::exporter::OpenMetricsExporter;
use opentelemetry_sdk::metrics::PeriodicReader;
use opentelemetry_sdk::metrics::SdkMeterProvider;

pub fn init_openmetrics_exporter() -> OpenMetricsExporter {
    let exporter = OpenMetricsExporter::default();
    let reader = PeriodicReader::builder(exporter.clone())
        .with_interval(Duration::from_secs(1))
        .build();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .build();
    opentelemetry::global::set_meter_provider(meter_provider);
    exporter
}

let exporter = init_openmetrics_exporter();
// Retain the exporter in you app state. Register some opentelmetry meters and fill them with data.
// Later on (e.g. in a `/metrics` endpoint) read the current metrics:
let openmetrics = exporter.text();
println!("{}", openmetrics);
```

## Tracey spec tracking

This project uses [Tracey](https://tracey.bearcove.eu/) to track which requirements of its
source specifications are implemented and tested. The configuration lives in
`.config/tracey/config.styx` and tracks two specification documents:

- `docs/spec/openmetrics.md`, condensed from the [OpenMetrics text format](https://github.com/prometheus/OpenMetrics/blob/main/specification/OpenMetrics.md)
  spec. Annotations use the `om[...]` prefix.
- `docs/spec/conversion.md`, condensed from the [OTLP metric points to Prometheus](https://github.com/open-telemetry/opentelemetry-specification/blob/v1.45.0/specification/compatibility/prometheus_and_openmetrics.md)
  conversion spec. Annotations use the `c[...]` prefix.

Spec requirements are referenced inline in the code via comment annotations:

```text
// om[impl text.eof]        // this code implements a requirement
// om[verify text.eof]      // this test verifies a requirement
// c[related metadata.unit-suffix]  // loosely connected
```

Useful commands:

- `mise run tracey-status` coverage overview per spec/implementation.
- `mise run tracey-validate` validate annotation references and naming.
- `tracey web` interactive coverage dashboard.
- `tracey uncovered` / `tracey untested` list the remaining work.
