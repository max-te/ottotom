//! Benchmarks of the OpenTelemetry to OpenMetrics text conversion, measured by CodSpeed.
//!
//! Run locally with `cargo bench --bench conversion`, or through CodSpeed with
//! `cargo codspeed build --bench conversion && codspeed run -- cargo codspeed run`.

use std::hint::black_box;

use divan::Bencher;
use ottotom::convert::{Config, WriteOpenMetrics};
use ottotom_testsupport::resource_metrics::{
    counter_metrics, gauge_metrics, histogram_metrics, make_large_test_metrics, make_test_metrics,
};

fn main() {
    divan::main();
}

/// Number of distinct attribute sets used by the scaling benchmarks.
const POINT_COUNTS: [usize; 3] = [16, 256, 2048];

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
