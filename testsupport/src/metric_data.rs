use opentelemetry::KeyValue;
use opentelemetry::metrics::MeterProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, Gauge, Histogram, MetricData, ResourceMetrics, Sum,
};
use opentelemetry_sdk::metrics::reader::MetricReader;

use crate::reader::TestMetricsReader;

fn extract_metric<'m>(
    metrics: &'m ResourceMetrics,
    scope_name: &str,
    metric_name: &str,
) -> Option<&'m AggregatedMetrics> {
    metrics
        .scope_metrics()
        .find_map(|m| (m.scope().name() == scope_name).then_some(m.metrics()))?
        .into_iter()
        .find(|m| m.name() == metric_name)
        .map(|g| g.data())
}

pub fn make_f64_gauge_metric(values: Vec<(f64, Vec<KeyValue>)>) -> Gauge<f64> {
    let reader = TestMetricsReader::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader.clone())
        .build();
    let scope_name = "test_meter";
    let meter = meter_provider.meter(scope_name);

    const MYGAUGE: &str = "mygauge";
    let gauge_builder = meter.f64_gauge(MYGAUGE);
    let gauge = gauge_builder.build();

    // Record all values with their attributes
    for (value, attrs) in values {
        gauge.record(value, attrs.as_slice());
    }

    // Collect metrics
    let mut metrics = ResourceMetrics::default();
    reader.collect(&mut metrics).unwrap();

    // Extract the gauge data
    let Some(AggregatedMetrics::F64(MetricData::Gauge(gauge))) =
        extract_metric(&metrics, scope_name, MYGAUGE)
    else {
        panic!("MYGAUGE should be f64 gauge");
    };
    gauge.clone()
}

#[test]
fn test_make_f64_gauge_metric() {
    let values = &[(2.5, vec![KeyValue::new("key", "value")]), (-3.0, vec![])];
    let gauge = make_f64_gauge_metric(values.to_vec());

    assert_eq!(gauge.data_points().count(), values.len());
    assert_eq!(
        gauge
            .data_points()
            .map(|dp| dp.attributes().count())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        gauge.data_points().map(|dp| dp.value()).sum::<f64>(),
        values.iter().map(|v| v.0).sum()
    );
}

pub fn make_u64_gauge_metric(values: Vec<(u64, Vec<KeyValue>)>) -> Gauge<u64> {
    let reader = TestMetricsReader::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader.clone())
        .build();
    let scope_name = "test_meter";
    let meter = meter_provider.meter(scope_name);

    const MYGAUGE: &str = "mygauge";
    let gauge_builder = meter.u64_gauge(MYGAUGE);
    let gauge = gauge_builder.build();

    // Record all values with their attributes
    for (value, attrs) in values {
        gauge.record(value, attrs.as_slice());
    }

    // Collect metrics
    let mut metrics = ResourceMetrics::default();
    reader.collect(&mut metrics).unwrap();

    // Extract the gauge data
    let Some(AggregatedMetrics::U64(MetricData::Gauge(gauge))) =
        extract_metric(&metrics, scope_name, MYGAUGE)
    else {
        panic!("MYGAUGE should be u64 gauge");
    };
    gauge.clone()
}

#[test]
fn test_make_u64_gauge_metric() {
    let values = &[(2, vec![KeyValue::new("key", "value")]), (3, vec![])];
    let gauge = make_u64_gauge_metric(values.to_vec());

    assert_eq!(gauge.data_points().count(), values.len());
    assert_eq!(
        gauge
            .data_points()
            .map(|dp| dp.attributes().count())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        gauge.data_points().map(|dp| dp.value()).sum::<u64>(),
        values.iter().map(|v| v.0).sum()
    );
}

pub fn make_i64_gauge_metric(values: Vec<(i64, Vec<KeyValue>)>) -> Gauge<i64> {
    let reader = TestMetricsReader::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader.clone())
        .build();
    let scope_name = "test_meter";
    let meter = meter_provider.meter(scope_name);

    const MYGAUGE: &str = "mygauge";
    let gauge_builder = meter.i64_gauge(MYGAUGE);
    let gauge = gauge_builder.build();

    // Record all values with their attributes
    for (value, attrs) in values {
        gauge.record(value, attrs.as_slice());
    }

    // Collect metrics
    let mut metrics = ResourceMetrics::default();
    reader.collect(&mut metrics).unwrap();

    // Extract the gauge data
    let Some(AggregatedMetrics::I64(MetricData::Gauge(gauge))) =
        extract_metric(&metrics, scope_name, MYGAUGE)
    else {
        panic!("MYGAUGE should be i64 gauge");
    };
    gauge.clone()
}

#[test]
fn test_make_i64_gauge_metric() {
    let values = &[(2, vec![KeyValue::new("key", "value")]), (-3, vec![])];
    let gauge = make_i64_gauge_metric(values.to_vec());

    assert_eq!(gauge.data_points().count(), values.len());
    assert_eq!(
        gauge
            .data_points()
            .map(|dp| dp.attributes().count())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        gauge.data_points().map(|dp| dp.value()).sum::<i64>(),
        values.iter().map(|v| v.0).sum()
    );
}

pub fn make_u64_counter_metric(values: Vec<(u64, Vec<KeyValue>)>) -> Sum<u64> {
    let reader = TestMetricsReader::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader.clone())
        .build();
    let scope_name = "test_meter";
    let meter = meter_provider.meter(scope_name);

    const MYCOUNTER: &str = "mycounter";
    let counter_builder = meter.u64_counter(MYCOUNTER);
    let counter = counter_builder.build();

    // Record all values with their attributes
    for (value, attrs) in values {
        counter.add(value, attrs.as_slice());
    }

    // Collect metrics
    let mut metrics = ResourceMetrics::default();
    reader.collect(&mut metrics).unwrap();

    // Extract the sum data
    let Some(AggregatedMetrics::U64(MetricData::Sum(counter))) =
        extract_metric(&metrics, scope_name, MYCOUNTER)
    else {
        panic!("MYCOUNTER should be u64 sum");
    };
    counter.clone()
}

#[test]
fn test_make_u64_counter_metric() {
    let values = &[(2, vec![KeyValue::new("key", "value")]), (3, vec![])];
    let counter = make_u64_counter_metric(values.to_vec());

    assert_eq!(counter.data_points().count(), values.len());
    assert_eq!(
        counter
            .data_points()
            .map(|dp| dp.attributes().count())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        counter.data_points().map(|dp| dp.value()).sum::<u64>(),
        values.iter().map(|v| v.0).sum()
    );
}

pub fn make_f64_counter_metric(values: Vec<(f64, Vec<KeyValue>)>) -> Sum<f64> {
    let reader = TestMetricsReader::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader.clone())
        .build();
    let scope_name = "test_meter";
    let meter = meter_provider.meter(scope_name);

    const MYCOUNTER: &str = "mycounter";
    let counter_builder = meter.f64_counter(MYCOUNTER);
    let counter = counter_builder.build();

    // Record all values with their attributes
    for (value, attrs) in values {
        counter.add(value, attrs.as_slice());
    }

    // Collect metrics
    let mut metrics = ResourceMetrics::default();
    reader.collect(&mut metrics).unwrap();

    // Extract the sum data
    let Some(AggregatedMetrics::F64(MetricData::Sum(counter))) =
        extract_metric(&metrics, scope_name, MYCOUNTER)
    else {
        panic!("MYCOUNTER should be f64 sum");
    };
    counter.clone()
}

#[test]
fn test_make_f64_counter_metric() {
    let values = &[(2.5, vec![KeyValue::new("key", "value")]), (3.0, vec![])];
    let counter = make_f64_counter_metric(values.to_vec());

    assert_eq!(counter.data_points().count(), values.len());
    assert_eq!(
        counter
            .data_points()
            .map(|dp| dp.attributes().count())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        counter.data_points().map(|dp| dp.value()).sum::<f64>(),
        values.iter().map(|v| v.0).sum()
    );
}

pub fn make_i64_counter_metric(values: Vec<(i64, Vec<KeyValue>)>) -> Sum<i64> {
    let reader = TestMetricsReader::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader.clone())
        .build();
    let scope_name = "test_meter";
    let meter = meter_provider.meter(scope_name);

    const MYCOUNTER: &str = "mycounter";
    let counter_builder = meter.i64_up_down_counter(MYCOUNTER);
    let counter = counter_builder.build();

    // Record all values with their attributes
    for (value, attrs) in values {
        counter.add(value, attrs.as_slice());
    }

    // Collect metrics
    let mut metrics = ResourceMetrics::default();
    reader.collect(&mut metrics).unwrap();

    // Extract the sum data
    let Some(AggregatedMetrics::I64(MetricData::Sum(counter))) =
        extract_metric(&metrics, scope_name, MYCOUNTER)
    else {
        panic!("MYCOUNTER should be i64 sum");
    };
    counter.clone()
}

#[test]
fn test_make_i64_counter_metric() {
    let values = &[(2, vec![KeyValue::new("key", "value")]), (-3, vec![])];
    let counter = make_i64_counter_metric(values.to_vec());

    assert_eq!(counter.data_points().count(), values.len());
    assert_eq!(
        counter
            .data_points()
            .map(|dp| dp.attributes().count())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        counter.data_points().map(|dp| dp.value()).sum::<i64>(),
        values.iter().map(|v| v.0).sum()
    );
}

pub fn make_f64_histogram_metric(values: Vec<(f64, Vec<KeyValue>)>) -> Histogram<f64> {
    let reader = TestMetricsReader::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader.clone())
        .build();
    let scope_name = "test_meter";
    let meter = meter_provider.meter(scope_name);

    const MYHISTOGRAM: &str = "myhistogram";
    let histogram_builder = meter.f64_histogram(MYHISTOGRAM);
    let histogram = histogram_builder.build();

    // Record all values with their attributes
    for (value, attrs) in values {
        histogram.record(value, attrs.as_slice());
    }

    // Collect metrics
    let mut metrics = ResourceMetrics::default();
    reader.collect(&mut metrics).unwrap();

    // Extract the histogram data
    let Some(AggregatedMetrics::F64(MetricData::Histogram(histogram))) =
        extract_metric(&metrics, scope_name, MYHISTOGRAM)
    else {
        panic!("MYCOUNTER should be f64 histogram");
    };
    histogram.clone()
}

#[test]
fn test_make_f64_histogram_metric() {
    let values = &[(2.5, vec![KeyValue::new("key", "value")]), (3.0, vec![])];
    let histogram = make_f64_histogram_metric(values.to_vec());

    assert_eq!(histogram.data_points().count(), values.len());
    assert_eq!(
        histogram
            .data_points()
            .map(|dp| dp.attributes().count())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        histogram.data_points().map(|dp| dp.sum()).sum::<f64>(),
        values.iter().map(|v| v.0).sum()
    );
    assert_eq!(
        histogram
            .data_points()
            .map(|dp| dp.min().unwrap())
            .sum::<f64>(),
        values.iter().map(|v| v.0).sum()
    );
    assert_eq!(
        histogram
            .data_points()
            .map(|dp| dp.max().unwrap())
            .sum::<f64>(),
        values.iter().map(|v| v.0).sum()
    );
}

pub fn make_u64_histogram_metric(values: Vec<(u64, Vec<KeyValue>)>) -> Histogram<u64> {
    let reader = TestMetricsReader::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader.clone())
        .build();
    let scope_name = "test_meter";
    let meter = meter_provider.meter(scope_name);

    const MYHISTOGRAM: &str = "myhistogram";
    let histogram_builder = meter.u64_histogram(MYHISTOGRAM);
    let histogram = histogram_builder.build();

    // Record all values with their attributes
    for (value, attrs) in values {
        histogram.record(value, attrs.as_slice());
    }

    // Collect metrics
    let mut metrics = ResourceMetrics::default();
    reader.collect(&mut metrics).unwrap();

    // Extract the histogram data
    let Some(AggregatedMetrics::U64(MetricData::Histogram(histogram))) =
        extract_metric(&metrics, scope_name, MYHISTOGRAM)
    else {
        panic!("MYCOUNTER should be u64 histogram");
    };
    histogram.clone()
}

#[test]
fn test_make_u64_histogram_metric() {
    let values = &[(2, vec![KeyValue::new("key", "value")]), (3, vec![])];
    let histogram = make_u64_histogram_metric(values.to_vec());

    assert_eq!(histogram.data_points().count(), values.len());
    assert_eq!(
        histogram
            .data_points()
            .map(|dp| dp.attributes().count())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        histogram.data_points().map(|dp| dp.sum()).sum::<u64>(),
        values.iter().map(|v| v.0).sum()
    );
    assert_eq!(
        histogram
            .data_points()
            .map(|dp| dp.min().unwrap())
            .sum::<u64>(),
        values.iter().map(|v| v.0).sum()
    );
    assert_eq!(
        histogram
            .data_points()
            .map(|dp| dp.max().unwrap())
            .sum::<u64>(),
        values.iter().map(|v| v.0).sum()
    );
}
