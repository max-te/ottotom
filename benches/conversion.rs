//! Benchmarks of the OpenTelemetry to OpenMetrics text conversion, measured by CodSpeed.
//!
//! Run locally with `cargo bench --bench conversion`, or through CodSpeed with
//! `cargo codspeed build --bench conversion && codspeed run -- cargo codspeed run`.

use std::hint::black_box;

use divan::Bencher;
use opentelemetry::metrics::MeterProvider;
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::reader::MetricReader;
use ottotom::convert::{Config, WriteOpenMetrics};
use ottotom_testsupport::reader::TestMetricsReader;
use ottotom_testsupport::resource_metrics::{make_large_test_metrics, make_test_metrics};

fn main() {
    divan::main();
}

/// Number of distinct attribute sets used by the scaling benchmarks.
const POINT_COUNTS: [usize; 3] = [16, 256, 2048];

/// Builds a meter provider fed by a manual reader and collects whatever the
/// `record` closure has instrumented into a [`ResourceMetrics`] snapshot.
fn collect_metrics(record: impl FnOnce(&opentelemetry::metrics::Meter)) -> ResourceMetrics {
    let reader = TestMetricsReader::default();
    let meter_provider = SdkMeterProvider::builder()
        // `builder_empty` keeps the payload independent of the environment
        // (`OTEL_RESOURCE_ATTRIBUTES`), so measurements stay comparable.
        .with_resource(
            Resource::builder_empty()
                .with_service_name("ottotom-bench")
                .build(),
        )
        .with_reader(reader.clone())
        .build();
    let meter = meter_provider.meter_with_scope(
        InstrumentationScope::builder("ottotom.bench")
            .with_version("0.1.0")
            .with_attributes([KeyValue::new("scopek", "scopev")])
            .build(),
    );

    record(&meter);

    let mut metrics = ResourceMetrics::default();
    reader.collect(&mut metrics).unwrap();
    metrics
}

fn counter_metrics(points: usize) -> ResourceMetrics {
    collect_metrics(|meter| {
        let counter = meter
            .u64_counter("http.server.requests")
            .with_unit("s")
            .with_description("Number of handled requests")
            .build();
        for i in 0..points {
            counter.add(
                42 * i as u64,
                &[
                    KeyValue::new("http.route", format!("/api/v1/resource/{i}")),
                    KeyValue::new("http.response.status_code", 200),
                ],
            );
        }
    })
}

fn gauge_metrics(points: usize) -> ResourceMetrics {
    collect_metrics(|meter| {
        let gauge = meter
            .f64_gauge("system.memory.utilization")
            .with_description("Memory utilization")
            .build();
        for i in 0..points {
            gauge.record(
                4.2 * i as f64,
                &[KeyValue::new("system.device", format!("dev{i}"))],
            );
        }
    })
}

fn histogram_metrics(points: usize) -> ResourceMetrics {
    collect_metrics(|meter| {
        let histogram = meter
            .f64_histogram("http.server.duration")
            .with_unit("s")
            .with_description("Request duration")
            .build();
        for i in 0..points {
            histogram.record(
                4.22 / (i + 1) as f64,
                &[
                    KeyValue::new("http.route", format!("/api/v1/resource/{i}")),
                    KeyValue::new("http.request.method", "GET"),
                ],
            );
        }
    })
}

/// Full conversion of a small, mixed set of metrics into a fresh [`String`].
#[divan::bench]
fn small_to_string(bencher: Bencher) {
    let metrics = make_test_metrics();

    bencher.bench(|| black_box(&metrics).to_openmetrics_string().unwrap());
}

/// Full conversion of the large mixed metric set into a fresh [`String`].
#[divan::bench]
fn large_to_string(bencher: Bencher) {
    let metrics = make_large_test_metrics();

    bencher.bench(|| black_box(&metrics).to_openmetrics_string().unwrap());
}

/// Conversion into an already allocated buffer, as done by the exporter.
#[divan::bench]
fn large_into_reused_buffer(bencher: Bencher) {
    let metrics = make_large_test_metrics();
    let capacity = metrics.to_openmetrics_string().unwrap().len();

    bencher
        .with_inputs(|| String::with_capacity(capacity))
        .bench_refs(|buffer| {
            buffer.clear();
            black_box(&metrics).write_as_openmetrics(buffer).unwrap();
        });
}

/// Same payload, but with the `otel_scope_info` and `target_info` metrics disabled.
#[divan::bench]
fn large_without_scope_and_target_info(bencher: Bencher) {
    let metrics = make_large_test_metrics();
    let config = Config::builder()
        .scope_info_enabled(false)
        .target_info_enabled(false)
        .build();

    bencher.bench(|| {
        black_box(&metrics)
            .to_openmetrics_string_with_config(black_box(config))
            .unwrap()
    });
}

/// Monotonic sums, scaled by the number of attribute sets.
#[divan::bench(args = POINT_COUNTS)]
fn counter(bencher: Bencher, points: usize) {
    let metrics = counter_metrics(points);

    bencher.bench(|| black_box(&metrics).to_openmetrics_string().unwrap());
}

/// Gauges, scaled by the number of attribute sets.
#[divan::bench(args = POINT_COUNTS)]
fn gauge(bencher: Bencher, points: usize) {
    let metrics = gauge_metrics(points);

    bencher.bench(|| black_box(&metrics).to_openmetrics_string().unwrap());
}

/// Histograms, scaled by the number of attribute sets. Each point expands into
/// one sample per bucket boundary, so this is the heaviest metric kind.
#[divan::bench(args = POINT_COUNTS)]
fn histogram(bencher: Bencher, points: usize) {
    let metrics = histogram_metrics(points);

    bencher.bench(|| black_box(&metrics).to_openmetrics_string().unwrap());
}
