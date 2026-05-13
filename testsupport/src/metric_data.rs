use opentelemetry::KeyValue;
use opentelemetry::metrics::{Meter, MeterProvider};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, Gauge, Histogram, MetricData, ResourceMetrics, Sum,
};
use opentelemetry_sdk::metrics::reader::MetricReader;

use crate::reader::TestMetricsReader;

struct TestMeter {
    reader: TestMetricsReader,
    _provider: SdkMeterProvider,
    scope: &'static str,
    meter: Meter,
}

impl TestMeter {
    fn new() -> Self {
        let reader = TestMetricsReader::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(reader.clone())
            .build();
        let scope = "test_meter";
        let meter = provider.meter(scope);

        Self {
            reader,
            _provider: provider, // When the provider is dropped the reader is shut down.
            scope,
            meter,
        }
    }

    fn extract<T: FromAggregated + Clone>(&self, metric_name: &str) -> Option<T> {
        let mut metrics = ResourceMetrics::default();
        self.reader.collect(&mut metrics).unwrap();
        metrics
            .scope_metrics()
            .find_map(|m| (m.scope().name() == self.scope).then_some(m.metrics()))?
            .find(|m| m.name() == metric_name)
            .map(|g| g.data())
            .and_then(T::from_aggregated)
            .cloned()
    }
}

trait FromAggregated {
    fn from_aggregated(metrics: &AggregatedMetrics) -> Option<&Self>;
}

macro_rules! impl_from_aggregated {
    ($aggregatedMetricsVariant:ident, $metricDataVariant:ident for $($For:tt)*) => {
        impl FromAggregated for $($For)* {
            fn from_aggregated(metrics: &AggregatedMetrics) -> Option<&Self> {
                match metrics {
                    AggregatedMetrics::$aggregatedMetricsVariant(MetricData::$metricDataVariant(it)) => Some(it),
                    _ => None,
                }
            }
        }

    };
}

trait MakeMetric {
    type ValueType;
    fn make_metric<I: IntoIterator<Item = (Self::ValueType, Vec<KeyValue>)>>(
        observations: I,
    ) -> Self;
}

macro_rules! impl_make_metric {
    ($ValueType:ident, $instrumentMethod:ident, $recordMethod:ident for $($For:tt)*) => {
        impl MakeMetric for $($For)* {
            type ValueType = $ValueType;

            fn make_metric<'a, I: IntoIterator<Item = (Self::ValueType, Vec<KeyValue>)>>(observations: I) -> Self {
                let testmeter = TestMeter::new();
                let name = concat!("my_", stringify!($instrumentMethod));
                let instrument = testmeter.meter.$instrumentMethod(name).build();
                for (value, attrs) in observations {
                    instrument.$recordMethod(value, attrs.as_slice());
                }
                testmeter.extract::<Self>(name).unwrap()
            }
        }

    };
}

impl_from_aggregated!(F64, Gauge for Gauge<f64>);
impl_make_metric!(f64, f64_gauge, record for Gauge<f64>);

pub fn make_f64_gauge_metric(values: Vec<(f64, Vec<KeyValue>)>) -> Gauge<f64> {
    Gauge::<f64>::make_metric(values)
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

impl_from_aggregated!(U64, Gauge for Gauge<u64>);
impl_make_metric!(u64, u64_gauge, record for Gauge<u64>);

pub fn make_u64_gauge_metric(values: Vec<(u64, Vec<KeyValue>)>) -> Gauge<u64> {
    Gauge::<u64>::make_metric(values)
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

impl_from_aggregated!(I64, Gauge for Gauge<i64>);
impl_make_metric!(i64, i64_gauge, record for Gauge<i64>);

pub fn make_i64_gauge_metric(values: Vec<(i64, Vec<KeyValue>)>) -> Gauge<i64> {
    Gauge::<i64>::make_metric(values)
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

impl_from_aggregated!(U64, Sum for Sum<u64>);
impl_make_metric!(u64, u64_counter, add for Sum<u64>);

pub fn make_u64_counter_metric(values: Vec<(u64, Vec<KeyValue>)>) -> Sum<u64> {
    Sum::<u64>::make_metric(values)
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

impl_from_aggregated!(F64, Sum for Sum<f64>);
impl_make_metric!(f64, f64_counter, add for Sum<f64>);

pub fn make_f64_counter_metric(values: Vec<(f64, Vec<KeyValue>)>) -> Sum<f64> {
    Sum::<f64>::make_metric(values)
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

impl_from_aggregated!(I64, Sum for Sum<i64>);
impl_make_metric!(i64, i64_up_down_counter, add for Sum<i64>);

pub fn make_i64_counter_metric(values: Vec<(i64, Vec<KeyValue>)>) -> Sum<i64> {
    Sum::<i64>::make_metric(values)
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

impl_from_aggregated!(F64, Histogram for Histogram<f64>);
impl_make_metric!(f64, f64_histogram, record for Histogram<f64>);

pub fn make_f64_histogram_metric(values: Vec<(f64, Vec<KeyValue>)>) -> Histogram<f64> {
    Histogram::<f64>::make_metric(values)
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

impl_from_aggregated!(U64, Histogram for Histogram<u64>);
impl_make_metric!(u64, u64_histogram, record for Histogram<u64>);

pub fn make_u64_histogram_metric(values: Vec<(u64, Vec<KeyValue>)>) -> Histogram<u64> {
    Histogram::<u64>::make_metric(values)
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
