use crate::reader::TestMetricsReader;
use opentelemetry::KeyValue;
use opentelemetry::metrics::MeterProvider;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::reader::MetricReader;

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
    let meter = meter_provider.meter("meter.1");

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
