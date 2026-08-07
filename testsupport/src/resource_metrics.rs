use crate::reader::TestMetricsReader;
use opentelemetry::metrics::{Meter, MeterProvider};
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::reader::MetricReader;

/// Builds a meter provider fed by a manual reader and collects whatever the
/// `record` closure has instrumented into a [`ResourceMetrics`] snapshot.
///
/// The resource is built with [`Resource::builder_empty`] so the payload is
/// independent of the environment (`OTEL_RESOURCE_ATTRIBUTES`), keeping test
/// and benchmark measurements comparable.
pub fn collect_metrics(record: impl FnOnce(&Meter)) -> ResourceMetrics {
    let reader = TestMetricsReader::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_resource(
            Resource::builder_empty()
                .with_service_name("ottotom-testsupport")
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

/// A monotonic sum named `http.server.requests`, scaled to `points` distinct
/// attribute sets.
pub fn counter_metrics(points: usize) -> ResourceMetrics {
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

/// A gauge named `system.memory.utilization`, scaled to `points` distinct
/// attribute sets.
pub fn gauge_metrics(points: usize) -> ResourceMetrics {
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

/// A histogram named `http.server.duration`, scaled to `points` distinct
/// attribute sets.
pub fn histogram_metrics(points: usize) -> ResourceMetrics {
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

pub fn make_test_metrics() -> ResourceMetrics {
    let reader = TestMetricsReader::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_resource(
            Resource::builder()
                .with_service_name("ottotom-testsupport")
                .build(),
        )
        .with_reader(reader.clone())
        .build();
    let meter = meter_provider.meter_with_scope(
        InstrumentationScope::builder("meter.1")
            .with_version("0.0.1")
            .with_schema_url("http://example.com/schema")
            .with_attributes([KeyValue::new("scopek", "scopev")])
            .build(),
    );

    let gauge = meter
        .f64_gauge("f64.gauge")
        .with_description("A \"gauge\"\nFor testing")
        .build();
    gauge.record(4.2, &[KeyValue::new("kk", "v1")]);
    gauge.record(4.22, &[KeyValue::new("kk", "v1")]);
    gauge.record(4.23, &[KeyValue::new("kk", "v2")]);

    let counter = meter.u64_counter("u64.counter").with_unit("s").build();
    counter.add(125, &[]);

    let hist = meter.f64_histogram("histo").build();
    hist.record(0.0, &[]);
    hist.record(1.3, &[]);
    hist.record(1.4, &[]);
    hist.record(13.0, &[]);

    let mut metrics = ResourceMetrics::default();
    reader.collect(&mut metrics).unwrap();

    metrics
}

pub fn make_large_test_metrics() -> ResourceMetrics {
    let reader = TestMetricsReader::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader.clone())
        .build();
    let meter = meter_provider.meter("meter.1");

    let gauge = meter
        .f64_gauge("f64.gauge")
        .with_description("A \"gauge\"\nFor testing")
        .build();
    for i in 0..100 {
        gauge.record(4.22, &[KeyValue::new("foo.bar", format!("a{i}"))]);
    }

    let counter = meter.u64_counter("u64.counter").with_unit("s").build();
    for i in 0..1000 {
        counter.add(422 * i, &[KeyValue::new("high-low", format!("v\n{i}"))]);
    }

    let hist = meter.f64_histogram("histo").build();
    for i in 0..1000 {
        hist.record(
            4.22 / i as f64,
            &[
                KeyValue::new("x.y.z", format!("v{i}")),
                KeyValue::new("z.z.z", "fixed"),
                KeyValue::new("z.y.z", format!("0{}0", i + 1)),
            ],
        );
    }

    let mut metrics = ResourceMetrics::default();
    reader.collect(&mut metrics).unwrap();

    metrics
}
