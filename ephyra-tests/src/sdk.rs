//! Replays a [`Scenario`] through a real `opentelemetry_sdk` pipeline.

use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter, MeterProvider, UpDownCounter};
use opentelemetry::{InstrumentationScope, KeyValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::reader::MetricReader;
use ottotom::convert::Config;
use ottotom_testsupport::reader::TestMetricsReader;

use crate::scenario::{Attr, AttrValue, Instrument, Kind, Record, Scenario};

/// The order in which a scenario is replayed. Both orders describe the same
/// measurements, so they must convert to the same exposition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    AsWritten,
    /// Meters and instruments are created in reverse, and every record
    /// passes its attributes in reverse. Measurements keep their order,
    /// because a gauge keeps the last one.
    Reversed,
}

pub fn config(scenario: &Scenario) -> Config {
    Config::builder()
        .scope_info_enabled(scenario.scope_info)
        .target_info_enabled(scenario.target_info)
        .build()
}

pub fn collect(scenario: &Scenario, order: Order) -> ResourceMetrics {
    let reader = TestMetricsReader::default();
    let provider = SdkMeterProvider::builder()
        .with_resource(
            Resource::builder_empty()
                .with_attributes(key_values(&scenario.resource, Order::AsWritten))
                .build(),
        )
        .with_reader(reader.clone())
        .build();

    let positions: Vec<(usize, usize)> = scenario
        .scopes
        .iter()
        .enumerate()
        .flat_map(|(s, scope)| (0..scope.instruments.len()).map(move |i| (s, i)))
        .collect();
    let mut handles: Vec<Option<Handle>> = positions.iter().map(|_| None).collect();
    // One meter per scenario scope, as an application holds one per library.
    let mut meters: Vec<Option<Meter>> = scenario.scopes.iter().map(|_| None).collect();
    let creation_order: Vec<usize> = match order {
        Order::AsWritten => (0..positions.len()).collect(),
        Order::Reversed => (0..positions.len()).rev().collect(),
    };
    for slot in creation_order {
        let (s, i) = positions[slot];
        let scope = &scenario.scopes[s];
        let meter = meters[s].get_or_insert_with(|| {
            let mut builder = InstrumentationScope::builder(scope.name.clone())
                .with_attributes(key_values(&scope.attrs, Order::AsWritten));
            if let Some(version) = &scope.version {
                builder = builder.with_version(version.clone());
            }
            provider.meter_with_scope(builder.build())
        });
        handles[slot] = Some(Handle::create(meter, &scope.instruments[i]));
    }

    for (slot, &(s, i)) in positions.iter().enumerate() {
        let handle = handles[slot]
            .as_ref()
            .expect("every instrument was created");
        for record in &scenario.scopes[s].instruments[i].records {
            handle.record(record, &key_values(&record.attrs, order));
        }
    }

    let mut metrics = ResourceMetrics::default();
    reader
        .collect(&mut metrics)
        .expect("the reader is registered");
    metrics
}

/// Converts attributes, keeping the last value of a repeated key because the
/// SDK's handling of repeated keys is not what these tests are about.
fn key_values(attrs: &[Attr], order: Order) -> Vec<KeyValue> {
    let mut unique: Vec<&Attr> = Vec::new();
    for attr in attrs.iter().rev() {
        if !unique.iter().any(|kept| kept.key == attr.key) {
            unique.push(attr);
        }
    }
    if order == Order::AsWritten {
        unique.reverse();
    }
    unique
        .into_iter()
        .map(|attr| {
            let value = match &attr.value {
                AttrValue::Str(s) => Value::from(s.clone()),
                AttrValue::I64(i) => Value::I64(*i),
                AttrValue::F64(f) => Value::F64(*f),
                AttrValue::Bool(b) => Value::Bool(*b),
            };
            KeyValue::new(attr.key.clone(), value)
        })
        .collect()
}

/// Finite, strictly increasing boundaries, which is what the SDK accepts.
pub fn valid_bounds(raw: &[f64]) -> Vec<f64> {
    let mut bounds: Vec<f64> = raw.iter().copied().filter(|b| b.is_finite()).collect();
    bounds.sort_by(f64::total_cmp);
    // -0.0 and 0.0 compare equal, and the SDK rejects equal neighbours.
    bounds.dedup_by(|a, b| a == b);
    bounds
}

/// A u64 histogram value small enough that the SDK's running sum cannot
/// overflow, because the SDK adds with `+` and these tests enable overflow checks.
fn histogram_u64(record: &Record) -> u64 {
    record.int.unsigned_abs() >> 8
}

enum Handle {
    U64Counter(Counter<u64>),
    F64Counter(Counter<f64>),
    I64UpDownCounter(UpDownCounter<i64>),
    F64UpDownCounter(UpDownCounter<f64>),
    U64Gauge(Gauge<u64>),
    I64Gauge(Gauge<i64>),
    F64Gauge(Gauge<f64>),
    U64Histogram(Histogram<u64>),
    F64Histogram(Histogram<f64>),
}

impl Handle {
    fn create(meter: &Meter, instrument: &Instrument) -> Self {
        let name = instrument.name.clone();
        let unit = instrument.unit.clone();
        let description = instrument.description.clone();
        let bounds = instrument.bounds.as_deref().map(valid_bounds);
        match instrument.kind {
            Kind::U64Counter => Handle::U64Counter(
                meter
                    .u64_counter(name)
                    .with_unit(unit)
                    .with_description(description)
                    .build(),
            ),
            Kind::F64Counter => Handle::F64Counter(
                meter
                    .f64_counter(name)
                    .with_unit(unit)
                    .with_description(description)
                    .build(),
            ),
            Kind::I64UpDownCounter => Handle::I64UpDownCounter(
                meter
                    .i64_up_down_counter(name)
                    .with_unit(unit)
                    .with_description(description)
                    .build(),
            ),
            Kind::F64UpDownCounter => Handle::F64UpDownCounter(
                meter
                    .f64_up_down_counter(name)
                    .with_unit(unit)
                    .with_description(description)
                    .build(),
            ),
            Kind::U64Gauge => Handle::U64Gauge(
                meter
                    .u64_gauge(name)
                    .with_unit(unit)
                    .with_description(description)
                    .build(),
            ),
            Kind::I64Gauge => Handle::I64Gauge(
                meter
                    .i64_gauge(name)
                    .with_unit(unit)
                    .with_description(description)
                    .build(),
            ),
            Kind::F64Gauge => Handle::F64Gauge(
                meter
                    .f64_gauge(name)
                    .with_unit(unit)
                    .with_description(description)
                    .build(),
            ),
            Kind::U64Histogram => {
                let mut builder = meter
                    .u64_histogram(name)
                    .with_unit(unit)
                    .with_description(description);
                if let Some(bounds) = bounds {
                    builder = builder.with_boundaries(bounds);
                }
                Handle::U64Histogram(builder.build())
            }
            Kind::F64Histogram => {
                let mut builder = meter
                    .f64_histogram(name)
                    .with_unit(unit)
                    .with_description(description);
                if let Some(bounds) = bounds {
                    builder = builder.with_boundaries(bounds);
                }
                Handle::F64Histogram(builder.build())
            }
        }
    }

    fn record(&self, record: &Record, attrs: &[KeyValue]) {
        match self {
            Handle::U64Counter(c) => c.add(record.int.unsigned_abs(), attrs),
            Handle::F64Counter(c) => c.add(record.float.abs(), attrs),
            Handle::I64UpDownCounter(c) => c.add(record.int, attrs),
            Handle::F64UpDownCounter(c) => c.add(record.float, attrs),
            Handle::U64Gauge(g) => g.record(record.int.unsigned_abs(), attrs),
            Handle::I64Gauge(g) => g.record(record.int, attrs),
            Handle::F64Gauge(g) => g.record(record.float, attrs),
            Handle::U64Histogram(h) => h.record(histogram_u64(record), attrs),
            Handle::F64Histogram(h) => h.record(record.float, attrs),
        }
    }
}
