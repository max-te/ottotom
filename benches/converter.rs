use std::hint::black_box;
use std::rc::Rc;

use opentelemetry_sdk::metrics::data::ResourceMetrics;
use ottotom::convert::{Config, WriteOpenMetrics};
use ottotom_testsupport::resource_metrics::{
    counter_metrics, gauge_metrics, histogram_metrics, make_large_test_metrics, make_test_metrics,
};
use tango_bench::{Benchmark, IntoBenchmarks, benchmark_fn, tango_benchmarks, tango_main};

/// Number of distinct attribute sets used by the scaling benchmarks.
const POINT_COUNTS: [usize; 3] = [16, 256, 2048];

pub fn benchmarks() -> impl IntoBenchmarks {
    let mut all = vec![
        // Full conversion of a small, mixed set of metrics into a fresh String.
        benchmark_fn("small_to_string", |b| {
            let metrics = Rc::new(make_test_metrics());
            b.iter(move || black_box(&metrics).to_openmetrics_string().unwrap())
        }),
        // Full conversion of the large mixed metric set into a fresh String.
        benchmark_fn("large_to_string", |b| {
            let metrics = Rc::new(make_large_test_metrics());
            b.iter(move || black_box(&metrics).to_openmetrics_string().unwrap())
        }),
        // Conversion into an already allocated buffer, as done by the exporter.
        benchmark_fn("large_into_reused_buffer", |b| {
            let metrics = Rc::new(make_large_test_metrics());
            let mut buffer = String::new();
            b.iter(move || {
                buffer.clear();
                black_box(&metrics).write_as_openmetrics(black_box(&mut buffer))
            })
        }),
        // Same payload, but with the `otel_scope_info` and `target_info` metrics disabled.
        benchmark_fn("large_without_scope_and_target_info", |b| {
            let metrics = Rc::new(make_large_test_metrics());
            let config = Config::builder()
                .scope_info_enabled(false)
                .target_info_enabled(false)
                .build();
            b.iter(move || {
                black_box(&metrics)
                    .to_openmetrics_string_with_config(black_box(config))
                    .unwrap()
            })
        }),
    ];
    all.extend(scaling_benchmarks());
    all
}

/// Monotonic sums, gauges and histograms, each scaled by the number of
/// attribute sets.
fn scaling_benchmarks() -> Vec<Benchmark> {
    let mut benchmarks = Vec::new();
    for &points in &POINT_COUNTS {
        benchmarks.push(benchmark_fn(
            format!("counter/{points}"),
            scale_bench(counter_metrics(points)),
        ));
        benchmarks.push(benchmark_fn(
            format!("gauge/{points}"),
            scale_bench(gauge_metrics(points)),
        ));
        benchmarks.push(benchmark_fn(
            format!("histogram/{points}"),
            scale_bench(histogram_metrics(points)),
        ));
    }
    benchmarks
}

fn scale_bench(
    metrics: ResourceMetrics,
) -> impl FnMut(tango_bench::Bencher) -> Box<dyn tango_bench::ErasedSampler> {
    let metrics = Rc::new(metrics);
    move |b| {
        let metrics = metrics.clone();
        b.iter(move || black_box(&metrics).to_openmetrics_string().unwrap())
    }
}

tango_benchmarks!(benchmarks());
tango_main!();
