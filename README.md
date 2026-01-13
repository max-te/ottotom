# opentelemetry-openmetrics

A Rust crate for exporting OpenTelemetry metrics in the [OpenMetrics](https://github.com/prometheus/OpenMetrics) text format.
This serves as a protobuf-free alternative to the discontinued `opentelemetry-prometheus` crate.

> ⚠️ **Warning:** This implementation is not fully spec-compliant for [OpenTelemetry-to-OpenMetrics conversion](https://github.com/open-telemetry/opentelemetry-specification/blob/v1.45.0/specification/compatibility/prometheus_and_openmetrics.md). Some edge cases and complex metrics setups may not be handled correctly. This library is still in an experimental state.

## Features

- **Conversion** of `opentelemetry-sdk` metric data to OpenMetrics-compliant text.
- **Ready-to-use Exporter** to register in `opentelemetry`, outputs metrics in the OpenMetrics text format.

## Usage

```rust,no_run
use std::time::Duration;
use opentelemetry_openmetrics::exporter::OpenMetricsExporter;
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
async {
    let openmetrics = exporter.text().await;
    println!("{}", openmetrics);
};
```
