use std::ops::Deref;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Meter, MeterProvider};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, ExponentialHistogram, Gauge, Histogram, Metric, MetricData, ResourceMetrics,
    Sum,
};
use opentelemetry_sdk::metrics::reader::MetricReader;
use opentelemetry_sdk::metrics::{Aggregation, InstrumentKind, MeterProviderBuilder, Stream};

use crate::reader::TestMetricsReader;

struct TestMeter {
    reader: TestMetricsReader,
    _provider: SdkMeterProvider,
    meter: Meter,
}

impl TestMeter {
    fn new() -> Self {
        Self::new_with(SdkMeterProvider::builder())
    }

    /// Configures the provider with a view that converts histogram instruments
    /// to the Base2 exponential histogram aggregation.
    ///
    /// See https://github.com/open-telemetry/opentelemetry-rust/issues/2111#issuecomment-3488799894
    fn new_exponential_histogram() -> Self {
        Self::new_with(SdkMeterProvider::builder().with_view(|inst| {
            if let InstrumentKind::Histogram = inst.kind() {
                Stream::builder()
                    .with_aggregation(Aggregation::Base2ExponentialHistogram {
                        max_size: 160,
                        max_scale: 20,
                        record_min_max: true,
                    })
                    .build()
                    .ok()
            } else {
                None
            }
        }))
    }

    fn new_with(builder: MeterProviderBuilder) -> Self {
        let reader = TestMetricsReader::default();
        let provider = builder.with_reader(reader.clone()).build();
        let meter = provider.meter("test_meter");

        Self {
            reader,
            _provider: provider, // When the provider is dropped the reader is shut down.
            meter,
        }
    }

    fn collect(&self) -> ResourceMetrics {
        let mut metrics = ResourceMetrics::default();
        self.reader.collect(&mut metrics).unwrap();
        metrics
    }
}

/// Owned handle to a collected [`Metric`] (name, description, unit and data).
///
/// `Metric` has no public constructor and is not `Clone`, so it can only be
/// borrowed from a collected [`ResourceMetrics`]. This handle keeps the
/// collected [`ResourceMetrics`] alive and derefs to the [`Metric`] it was
/// built from.
pub struct TestMetric {
    resource_metrics: ResourceMetrics,
    name: &'static str,
}

impl TestMetric {
    fn new(resource_metrics: ResourceMetrics, name: &'static str) -> Self {
        Self {
            resource_metrics,
            name,
        }
    }

    /// Returns a reference to the collected [`Metric`].
    pub fn metric(&self) -> &Metric {
        self.resource_metrics
            .scope_metrics()
            .find_map(|scope| scope.metrics().find(|m| m.name() == self.name))
            .expect("metric not found in collected resource metrics")
    }

    /// Returns a reference to the [`ResourceMetrics`] this handle was built from.
    pub fn resource_metrics(&self) -> &ResourceMetrics {
        &self.resource_metrics
    }

    /// Returns the [`ResourceMetrics`] this handle was built from.
    pub fn into_resource_metrics(self) -> ResourceMetrics {
        self.resource_metrics
    }

    /// Clones the metric's data out as `T` (e.g. `Gauge<f64>`, `Sum<u64>`,
    /// `Histogram<f64>`), or returns `None` if the data is of another kind.
    pub fn extract<T: FromAggregated + Clone>(&self) -> Option<T> {
        T::from_aggregated(self.metric().data()).cloned()
    }
}

impl Deref for TestMetric {
    type Target = Metric;

    fn deref(&self) -> &Self::Target {
        self.metric()
    }
}

/// Extract a metric's data from its [`AggregatedMetrics`] if it is of the
/// given type.
pub trait FromAggregated {
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

    /// Build a metric with the given name, unit, description and observations,
    /// and return an owned handle to it.
    ///
    /// The metric only appears in the collected [`ResourceMetrics`] if at least
    /// one observation is recorded (the SDK drops instruments without points).
    fn make_metric_handle<I: IntoIterator<Item = (Self::ValueType, Vec<KeyValue>)>>(
        name: &'static str,
        unit: Option<&'static str>,
        description: Option<&'static str>,
        observations: I,
    ) -> TestMetric;

    /// Build a metric and return its data (e.g. `Gauge<f64>`).
    fn make_metric<I: IntoIterator<Item = (Self::ValueType, Vec<KeyValue>)>>(
        observations: I,
    ) -> Self
    where
        Self: FromAggregated + Clone + Sized;
}

macro_rules! impl_make_metric {
    ($meterCtor:ident, $ValueType:ident, $instrumentMethod:ident, $recordMethod:ident for $($For:tt)*) => {
        impl MakeMetric for $($For)* {
            type ValueType = $ValueType;

            fn make_metric_handle<I: IntoIterator<Item = (Self::ValueType, Vec<KeyValue>)>>(
                name: &'static str,
                unit: Option<&'static str>,
                description: Option<&'static str>,
                observations: I,
            ) -> TestMetric {
                let testmeter = TestMeter::$meterCtor();
                let mut builder = testmeter.meter.$instrumentMethod(name);
                if let Some(unit) = unit {
                    builder = builder.with_unit(unit);
                }
                if let Some(description) = description {
                    builder = builder.with_description(description);
                }
                let instrument = builder.build();
                for (value, attrs) in observations {
                    instrument.$recordMethod(value, attrs.as_slice());
                }
                TestMetric::new(testmeter.collect(), name)
            }

            fn make_metric<I: IntoIterator<Item = (Self::ValueType, Vec<KeyValue>)>>(observations: I) -> Self
            where
                Self: FromAggregated + Clone + Sized,
            {
                Self::make_metric_handle(
                    concat!("my_", stringify!($instrumentMethod)),
                    None,
                    None,
                    observations,
                )
                .extract()
                .unwrap()
            }
        }

    };
}

impl_from_aggregated!(F64, Gauge for Gauge<f64>);
impl_make_metric!(new, f64, f64_gauge, record for Gauge<f64>);

pub fn make_f64_gauge_metric(values: Vec<(f64, Vec<KeyValue>)>) -> Gauge<f64> {
    Gauge::<f64>::make_metric(values)
}

pub fn make_f64_gauge_metric_handle(
    name: &'static str,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    values: Vec<(f64, Vec<KeyValue>)>,
) -> TestMetric {
    Gauge::<f64>::make_metric_handle(name, unit, description, values)
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

#[test]
fn test_make_f64_gauge_metric_handle() {
    let values = vec![(2.5, vec![KeyValue::new("key", "value")]), (-3.0, vec![])];
    let metric =
        make_f64_gauge_metric_handle("my_f64_gauge", Some("s"), Some("A gauge"), values.clone());

    assert_eq!(metric.name(), "my_f64_gauge");
    assert_eq!(metric.unit(), "s");
    assert_eq!(metric.description(), "A gauge");
    assert_eq!(metric.resource_metrics().scope_metrics().count(), 1);

    let gauge = metric.extract::<Gauge<f64>>().unwrap();
    assert_eq!(gauge.data_points().count(), values.len());
    assert_eq!(
        gauge.data_points().map(|dp| dp.value()).sum::<f64>(),
        values.iter().map(|v| v.0).sum()
    );
    assert!(metric.extract::<Sum<f64>>().is_none());
}

impl_from_aggregated!(U64, Gauge for Gauge<u64>);
impl_make_metric!(new, u64, u64_gauge, record for Gauge<u64>);

pub fn make_u64_gauge_metric(values: Vec<(u64, Vec<KeyValue>)>) -> Gauge<u64> {
    Gauge::<u64>::make_metric(values)
}

pub fn make_u64_gauge_metric_handle(
    name: &'static str,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    values: Vec<(u64, Vec<KeyValue>)>,
) -> TestMetric {
    Gauge::<u64>::make_metric_handle(name, unit, description, values)
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

#[test]
fn test_make_u64_gauge_metric_handle() {
    let values = vec![(2, vec![KeyValue::new("key", "value")]), (3, vec![])];
    let metric = make_u64_gauge_metric_handle("my_u64_gauge", None, None, values.clone());

    assert_eq!(metric.name(), "my_u64_gauge");
    let gauge = metric.extract::<Gauge<u64>>().unwrap();
    assert_eq!(gauge.data_points().count(), values.len());
    assert_eq!(
        gauge.data_points().map(|dp| dp.value()).sum::<u64>(),
        values.iter().map(|v| v.0).sum()
    );
}

impl_from_aggregated!(I64, Gauge for Gauge<i64>);
impl_make_metric!(new, i64, i64_gauge, record for Gauge<i64>);

pub fn make_i64_gauge_metric(values: Vec<(i64, Vec<KeyValue>)>) -> Gauge<i64> {
    Gauge::<i64>::make_metric(values)
}

pub fn make_i64_gauge_metric_handle(
    name: &'static str,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    values: Vec<(i64, Vec<KeyValue>)>,
) -> TestMetric {
    Gauge::<i64>::make_metric_handle(name, unit, description, values)
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

#[test]
fn test_make_i64_gauge_metric_handle() {
    let values = vec![(2, vec![KeyValue::new("key", "value")]), (-3, vec![])];
    let metric = make_i64_gauge_metric_handle("my_i64_gauge", None, None, values.clone());

    assert_eq!(metric.name(), "my_i64_gauge");
    let gauge = metric.extract::<Gauge<i64>>().unwrap();
    assert_eq!(gauge.data_points().count(), values.len());
    assert_eq!(
        gauge.data_points().map(|dp| dp.value()).sum::<i64>(),
        values.iter().map(|v| v.0).sum()
    );
}

impl_from_aggregated!(U64, Sum for Sum<u64>);
impl_make_metric!(new, u64, u64_counter, add for Sum<u64>);

pub fn make_u64_counter_metric(values: Vec<(u64, Vec<KeyValue>)>) -> Sum<u64> {
    Sum::<u64>::make_metric(values)
}

pub fn make_u64_counter_metric_handle(
    name: &'static str,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    values: Vec<(u64, Vec<KeyValue>)>,
) -> TestMetric {
    Sum::<u64>::make_metric_handle(name, unit, description, values)
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

#[test]
fn test_make_u64_counter_metric_handle() {
    let values = vec![(2, vec![KeyValue::new("key", "value")]), (3, vec![])];
    let metric = make_u64_counter_metric_handle("my_u64_counter", None, None, values.clone());

    assert_eq!(metric.name(), "my_u64_counter");
    let counter = metric.extract::<Sum<u64>>().unwrap();
    assert_eq!(counter.data_points().count(), values.len());
    assert_eq!(
        counter.data_points().map(|dp| dp.value()).sum::<u64>(),
        values.iter().map(|v| v.0).sum()
    );
}

impl_from_aggregated!(F64, Sum for Sum<f64>);
impl_make_metric!(new, f64, f64_counter, add for Sum<f64>);

pub fn make_f64_counter_metric(values: Vec<(f64, Vec<KeyValue>)>) -> Sum<f64> {
    Sum::<f64>::make_metric(values)
}

pub fn make_f64_counter_metric_handle(
    name: &'static str,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    values: Vec<(f64, Vec<KeyValue>)>,
) -> TestMetric {
    Sum::<f64>::make_metric_handle(name, unit, description, values)
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

#[test]
fn test_make_f64_counter_metric_handle() {
    let values = vec![(2.5, vec![KeyValue::new("key", "value")]), (3.0, vec![])];
    let metric = make_f64_counter_metric_handle("my_f64_counter", None, None, values.clone());

    assert_eq!(metric.name(), "my_f64_counter");
    let counter = metric.extract::<Sum<f64>>().unwrap();
    assert_eq!(counter.data_points().count(), values.len());
    assert_eq!(
        counter.data_points().map(|dp| dp.value()).sum::<f64>(),
        values.iter().map(|v| v.0).sum()
    );
}

impl_from_aggregated!(I64, Sum for Sum<i64>);
impl_make_metric!(new, i64, i64_up_down_counter, add for Sum<i64>);

pub fn make_i64_counter_metric(values: Vec<(i64, Vec<KeyValue>)>) -> Sum<i64> {
    Sum::<i64>::make_metric(values)
}

pub fn make_i64_counter_metric_handle(
    name: &'static str,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    values: Vec<(i64, Vec<KeyValue>)>,
) -> TestMetric {
    Sum::<i64>::make_metric_handle(name, unit, description, values)
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

#[test]
fn test_make_i64_counter_metric_handle() {
    let values = vec![(2, vec![KeyValue::new("key", "value")]), (-3, vec![])];
    let metric =
        make_i64_counter_metric_handle("my_i64_up_down_counter", None, None, values.clone());

    assert_eq!(metric.name(), "my_i64_up_down_counter");
    let counter = metric.extract::<Sum<i64>>().unwrap();
    assert_eq!(counter.data_points().count(), values.len());
    assert_eq!(
        counter.data_points().map(|dp| dp.value()).sum::<i64>(),
        values.iter().map(|v| v.0).sum()
    );
}

impl_from_aggregated!(F64, Histogram for Histogram<f64>);
impl_make_metric!(new, f64, f64_histogram, record for Histogram<f64>);

pub fn make_f64_histogram_metric(values: Vec<(f64, Vec<KeyValue>)>) -> Histogram<f64> {
    Histogram::<f64>::make_metric(values)
}

pub fn make_f64_histogram_metric_handle(
    name: &'static str,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    values: Vec<(f64, Vec<KeyValue>)>,
) -> TestMetric {
    Histogram::<f64>::make_metric_handle(name, unit, description, values)
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

#[test]
fn test_make_f64_histogram_metric_handle() {
    let values = vec![(2.5, vec![KeyValue::new("key", "value")]), (3.0, vec![])];
    let metric = make_f64_histogram_metric_handle("my_f64_histogram", None, None, values.clone());

    assert_eq!(metric.name(), "my_f64_histogram");
    let histogram = metric.extract::<Histogram<f64>>().unwrap();
    assert_eq!(histogram.data_points().count(), values.len());
    assert_eq!(
        histogram.data_points().map(|dp| dp.sum()).sum::<f64>(),
        values.iter().map(|v| v.0).sum()
    );
    assert!(metric.extract::<Gauge<f64>>().is_none());
}

impl_from_aggregated!(U64, Histogram for Histogram<u64>);
impl_make_metric!(new, u64, u64_histogram, record for Histogram<u64>);

pub fn make_u64_histogram_metric(values: Vec<(u64, Vec<KeyValue>)>) -> Histogram<u64> {
    Histogram::<u64>::make_metric(values)
}

pub fn make_u64_histogram_metric_handle(
    name: &'static str,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    values: Vec<(u64, Vec<KeyValue>)>,
) -> TestMetric {
    Histogram::<u64>::make_metric_handle(name, unit, description, values)
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

#[test]
fn test_make_u64_histogram_metric_handle() {
    let values = vec![(2, vec![KeyValue::new("key", "value")]), (3, vec![])];
    let metric = make_u64_histogram_metric_handle("my_u64_histogram", None, None, values.clone());

    assert_eq!(metric.name(), "my_u64_histogram");
    let histogram = metric.extract::<Histogram<u64>>().unwrap();
    assert_eq!(histogram.data_points().count(), values.len());
    assert_eq!(
        histogram.data_points().map(|dp| dp.sum()).sum::<u64>(),
        values.iter().map(|v| v.0).sum()
    );
}

impl_from_aggregated!(F64, ExponentialHistogram for ExponentialHistogram<f64>);
impl_make_metric!(
    new_exponential_histogram,
    f64,
    f64_histogram,
    record for ExponentialHistogram<f64>
);

pub fn make_f64_exponential_histogram_metric(
    values: Vec<(f64, Vec<KeyValue>)>,
) -> ExponentialHistogram<f64> {
    ExponentialHistogram::<f64>::make_metric(values)
}

pub fn make_f64_exponential_histogram_metric_handle(
    name: &'static str,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    values: Vec<(f64, Vec<KeyValue>)>,
) -> TestMetric {
    ExponentialHistogram::<f64>::make_metric_handle(name, unit, description, values)
}

#[test]
fn test_make_f64_exponential_histogram_metric() {
    let values = &[
        (2.5, vec![KeyValue::new("key", "value")]),
        (3.0, vec![KeyValue::new("key", "value")]),
        (25.0, vec![]),
    ];
    let histogram = make_f64_exponential_histogram_metric(values.to_vec());

    assert_eq!(histogram.data_points().count(), 2);
    for point in histogram.data_points() {
        assert!(point.scale() > 0);
        assert_eq!(
            point.count(),
            if point.attributes().count() == 0 {
                1
            } else {
                2
            }
        );
    }
    assert_eq!(
        histogram.data_points().map(|dp| dp.sum()).sum::<f64>(),
        values.iter().map(|v| v.0).sum()
    );
    assert_eq!(
        histogram.data_points().map(|dp| dp.count()).sum::<usize>(),
        values.len()
    );
}

#[test]
fn test_make_f64_exponential_histogram_metric_handle() {
    let values = vec![(2.5, vec![KeyValue::new("key", "value")]), (25.0, vec![])];
    let metric = make_f64_exponential_histogram_metric_handle(
        "my_exponential_histogram",
        Some("s"),
        Some("An exponential histogram"),
        values.clone(),
    );

    assert_eq!(metric.name(), "my_exponential_histogram");
    assert_eq!(metric.unit(), "s");
    assert_eq!(metric.description(), "An exponential histogram");
    assert_eq!(metric.resource_metrics().scope_metrics().count(), 1);

    let histogram = metric.extract::<ExponentialHistogram<f64>>().unwrap();
    assert_eq!(histogram.data_points().count(), values.len());
    assert_eq!(
        histogram.data_points().map(|dp| dp.sum()).sum::<f64>(),
        values.iter().map(|v| v.0).sum()
    );
    assert!(metric.extract::<Histogram<f64>>().is_none());
}

impl_from_aggregated!(U64, ExponentialHistogram for ExponentialHistogram<u64>);
impl_make_metric!(
    new_exponential_histogram,
    u64,
    u64_histogram,
    record for ExponentialHistogram<u64>
);

pub fn make_u64_exponential_histogram_metric(
    values: Vec<(u64, Vec<KeyValue>)>,
) -> ExponentialHistogram<u64> {
    ExponentialHistogram::<u64>::make_metric(values)
}

pub fn make_u64_exponential_histogram_metric_handle(
    name: &'static str,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    values: Vec<(u64, Vec<KeyValue>)>,
) -> TestMetric {
    ExponentialHistogram::<u64>::make_metric_handle(name, unit, description, values)
}

#[test]
fn test_make_u64_exponential_histogram_metric() {
    let values = &[
        (2, vec![KeyValue::new("key", "value")]),
        (3, vec![KeyValue::new("key", "value")]),
        (25, vec![]),
    ];
    let histogram = make_u64_exponential_histogram_metric(values.to_vec());

    assert_eq!(histogram.data_points().count(), 2);
    for point in histogram.data_points() {
        assert!(point.scale() > 0);
        assert_eq!(
            point.count(),
            if point.attributes().count() == 0 {
                1
            } else {
                2
            }
        );
    }
    assert_eq!(
        histogram.data_points().map(|dp| dp.sum()).sum::<u64>(),
        values.iter().map(|v| v.0).sum()
    );
    assert_eq!(
        histogram.data_points().map(|dp| dp.count()).sum::<usize>(),
        values.len()
    );
}

#[test]
fn test_make_u64_exponential_histogram_metric_handle() {
    let values = vec![(2, vec![KeyValue::new("key", "value")]), (25, vec![])];
    let metric = make_u64_exponential_histogram_metric_handle(
        "my_exponential_histogram",
        None,
        None,
        values.clone(),
    );

    assert_eq!(metric.name(), "my_exponential_histogram");
    let histogram = metric.extract::<ExponentialHistogram<u64>>().unwrap();
    assert_eq!(histogram.data_points().count(), values.len());
    assert_eq!(
        histogram.data_points().map(|dp| dp.sum()).sum::<u64>(),
        values.iter().map(|v| v.0).sum()
    );
}
