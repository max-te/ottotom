//! Writing exemplars, the sample measurements attached to a counter or bucket.

use std::time::SystemTime;

use opentelemetry::{KeyValue, SpanId, TraceId};
use opentelemetry_sdk::metrics::data::Exemplar;
use ufmt::{uWrite, uwrite};

use super::labels::write_attrs;
use super::to_timestamp;
use crate::format::Numeric;

/// Whether `value` belongs to the `i`-th bucket of `bounds`, which spans the
/// half-open range `(bounds[i - 1], bounds[i]]`. The index `bounds.len()`
/// denotes the `+Inf` bucket, holding everything above the last finite bound.
pub(super) fn falls_in_bucket<T: Numeric>(bounds: &[f64], i: usize, value: &T) -> bool {
    let value = value.to_f64();
    let lower = if i > 0 {
        bounds[i - 1]
    } else {
        f64::NEG_INFINITY
    };
    let upper = bounds.get(i).copied().unwrap_or(f64::INFINITY);
    value > lower && value <= upper
}

/// Writes the exemplar attached to the `i`-th bucket of `bounds`, if any.
pub(super) fn write_bucket_exemplar<'a, E: ExemplarPoint + 'a, U: uWrite>(
    f: &mut U,
    bounds: &[f64],
    i: usize,
    exemplars: impl IntoIterator<Item = &'a E>,
) -> Result<(), U::Error> {
    write_exemplar(
        f,
        exemplars
            .into_iter()
            .filter(|e| falls_in_bucket(bounds, i, &e.value())),
    )
}

/// The parts of an exemplar that the OpenMetrics encoding reads.
///
/// The SDK's [`Exemplar`] has no public constructor, so this is what lets a
/// test supply one of its own.
pub(crate) trait ExemplarPoint {
    type Value: Numeric + Copy;

    fn value(&self) -> Self::Value;
    fn time(&self) -> SystemTime;
    fn trace_id(&self) -> [u8; 16];
    fn span_id(&self) -> [u8; 8];
    fn filtered_attributes(&self) -> impl Iterator<Item = &KeyValue>;
}

// Plain forwarding, reachable only through exemplars the SDK produces - and
// this SDK produces none, leaving every mutant here unkillable.
#[cfg_attr(test, mutants::skip)]
impl<T: Numeric + Copy> ExemplarPoint for Exemplar<T> {
    type Value = T;

    fn value(&self) -> T {
        self.value
    }

    fn time(&self) -> SystemTime {
        Exemplar::time(self)
    }

    fn trace_id(&self) -> [u8; 16] {
        *Exemplar::trace_id(self)
    }

    fn span_id(&self) -> [u8; 8] {
        *Exemplar::span_id(self)
    }

    fn filtered_attributes(&self) -> impl Iterator<Item = &KeyValue> {
        Exemplar::filtered_attributes(self)
    }
}

pub(super) fn write_exemplar<'a, E: ExemplarPoint + 'a, U: uWrite>(
    f: &mut U,
    exemplars: impl IntoIterator<Item = &'a E>,
) -> Result<(), U::Error> {
    // Write at most one exemplar, preferring the most recent measurement.
    let Some(exemplar) = exemplars.into_iter().max_by_key(|e| e.time()) else {
        return Ok(());
    };
    // Build the exemplar's label set: the ids (only when a trace context was
    // active; the SDK zeroes them otherwise) plus the filtered attributes.
    // c[impl exemplar.trace-span-ids]
    let mut labels = Vec::new();
    let trace_id = exemplar.trace_id();
    if trace_id != [0; 16] {
        labels.push(KeyValue::new(
            "trace_id",
            TraceId::from_bytes(trace_id).to_string(),
        ));
    }
    let span_id = exemplar.span_id();
    if span_id != [0; 8] {
        labels.push(KeyValue::new(
            "span_id",
            SpanId::from_bytes(span_id).to_string(),
        ));
    }
    // c[impl exemplar.filtered-attrs] - filtered attributes become exemplar labels
    labels.extend(exemplar.filtered_attributes().cloned());

    // OpenMetrics exemplar: ` # {labels} value [timestamp]`
    // om[impl exemplars.structure] - an exemplar is a label set + value (+ timestamp)
    uwrite!(f, " # {{")?;
    // om[impl exemplars.empty-labelset] - an empty label set is rendered as `{}`
    write_attrs(f, labels.iter())?;
    // c[impl exemplar.timestamp] - timestamps are added as timestamps
    uwrite!(
        f,
        "}} {} {}",
        exemplar.value().fast_display(),
        to_timestamp(exemplar.time()),
    )?;
    Ok(())
}
