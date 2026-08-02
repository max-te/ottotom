use std::borrow::Cow;
use std::fmt::Write;
use std::hash::{DefaultHasher, Hasher};
use std::time::SystemTime;

use crate::format::Numeric;
use opentelemetry::{Key, KeyValue, Value};
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, Gauge, Histogram, MetricData, ResourceMetrics, Sum,
};
use opentelemetry_sdk::metrics::data::{Metric, ScopeMetrics};
use ufmt::{uDisplay, uWrite, uwriteln};
use unit::get_unit_suffixes;

#[cfg(test)]
mod tests;
mod unit;

/// The mime type of the text produced by this metrics formatter.
// om[impl text.contenttype]
pub const MIME_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// Trait to write the metrics data in OpenMetrics text format.
pub trait WriteOpenMetrics {
    /// Writes the metrics into `f` in OpenMetrics text format.
    fn write_as_openmetrics(&self, f: &mut impl Write) -> std::fmt::Result;
    /// Creates and returns a [String] of the metrics data in OpenMetrics text format.
    /// om[impl text.utf8] - output is always a valid UTF-8 String
    fn to_openmetrics_string(&self) -> Result<String, std::fmt::Error> {
        let mut out = String::new();
        self.write_as_openmetrics(&mut out)?;
        Ok(out)
    }
}

/// Serialization context for common variables needed during conversion.
struct Context<'f, W: uWrite> {
    /// the output [Write] reference
    f: W,
    /// a temporary buffer to store the serialized metric attributes
    attr_buffer: String,
    /// the sanitized name of the current metric
    name: String,
    /// the converted unit string of the current metric
    unit: Option<Cow<'static, str>>,
    /// the OpenMetrics metric type of the current metric
    typ: &'static str,
    /// the name of the current scope
    scope_name: &'f str,
    /// the version of the current scope
    scope_version: Option<&'f str>,
}

impl<'f, W: Write> Context<'f, WriteAsUWrite<'f, W>> {
    fn with_output(f: &'f mut W) -> Self {
        Context {
            f: WriteAsUWrite(f),
            attr_buffer: String::with_capacity(256),
            name: String::with_capacity(64),
            unit: None,
            typ: "",
            scope_name: "",
            scope_version: None,
        }
    }
}

struct WriteAsUWrite<'w, W: Write>(&'w mut W);

impl<W: Write> uWrite for WriteAsUWrite<'_, W> {
    type Error = std::fmt::Error;

    fn write_str(&mut self, s: &str) -> Result<(), Self::Error> {
        self.0.write_str(s)
    }

    fn write_char(&mut self, c: char) -> Result<(), Self::Error> {
        self.0.write_char(c)
    }
}

impl WriteOpenMetrics for ResourceMetrics {
    fn write_as_openmetrics(&self, f: &mut impl Write) -> std::fmt::Result {
        let mut ctx = Context::with_output(f);

        #[cfg(feature = "otel_scope_info")]
        write_target_info(&mut ctx.f, self.resource())?;

        let mut scopes: Vec<&ScopeMetrics> = self.scope_metrics().collect();
        scopes.sort_unstable_by_key(|s| s.scope().name());

        // c[impl scope.config-disable] - the cargo feature is the configuration switch
        #[cfg(feature = "otel_scope_info")]
        write_otel_scope_info(&mut ctx.f, &scopes)?;

        for scope in scopes {
            if cfg!(feature = "otel_scope_info") {
                ctx.scope_name = scope.scope().name();
                ctx.scope_version = scope.scope().version();
            }
            let mut metrics: Vec<_> = scope.metrics().collect();
            metrics.sort_unstable_by_key(|met| met.name());

            // om[impl metricfamily.nointerleave] - each MetricFamily is fully written before the next
            for metric in metrics {
                if extract_type_unit_and_name(&mut ctx, metric) {
                    write_header(&mut ctx, metric.description())?;
                    write_values(&mut ctx, metric.data())?;
                } else {
                    #[cfg(feature = "tracing")]
                    // c[impl metadata.drop-warn]
                    tracing::warn!("Unsupported metric type {metric:?}");
                }
            }
        }
        // om[impl text.eof]
        f.write_str("# EOF\n")?;
        Ok(())
    }
}

#[cfg(feature = "otel_scope_info")]
// c[impl resource.target-info]
fn write_target_info<U: uWrite>(
    f: &mut U,
    resource: &opentelemetry_sdk::Resource,
) -> Result<(), U::Error> {
    // c[impl resource.target-labels] - info-typed, only resource attributes
    f.write_str("# TYPE target info\n")?;
    // om[impl info.suffix]
    f.write_str("target_info{")?;
    // c[impl resource.attrs-sanitize] - resource attrs go through the same sanitization
    write_attrs_tuple(f, resource.iter())?;
    // om[impl info.value]
    f.write_str("} 1\n")?;
    Ok(())
}

fn extract_type_unit_and_name(
    ctx: &mut Context<'_, impl uWrite<Error = std::fmt::Error>>,
    metric: &Metric,
) -> bool {
    let Ok(typ) = get_type(metric.data()) else {
        return false;
    };
    ctx.typ = typ;
    ctx.unit = get_unit_suffixes(metric.unit());

    ctx.name.clear();
    let Ok(()) = write_sanitized_name(&mut ctx.name, metric.name(), NameKind::Metric);
    // om[impl metadata.unit-suffix]
    // c[impl metadata.unit-suffix]
    if let Some(ref unit) = ctx.unit
        && !ctx.name.ends_with(unit.as_ref())
    {
        ctx.name.push('_');
        ctx.name.push_str(unit);
    }

    true
}

/// Gets the OpenMetrics metric type for this [`AggregatedMetrics`].
/// Returns `Err(())` for unsupported metric types.
fn get_type(metric: &AggregatedMetrics) -> Result<&'static str, ()> {
    fn get_metric_data_type<T>(metric_data: &MetricData<T>) -> Result<&'static str, ()> {
        match metric_data {
            MetricData::Gauge(_) => Ok("gauge"),
            MetricData::Sum(sum) => {
                if sum.temporality() == Temporality::Cumulative {
                    if sum.is_monotonic() {
                        // c[impl sum.cumulative-monotonic]
                        Ok("counter")
                    } else {
                        // c[impl sum.cumulative-nonmonotonic.default]
                        Ok("gauge")
                    }
                } else {
                    // c[impl sum.drop]
                    Err(())
                }
            }
            MetricData::Histogram(hist) => {
                if hist.temporality() == Temporality::Cumulative {
                    // c[impl histogram.bucket]
                    Ok("histogram")
                } else {
                    // c[impl histogram.delta]
                    Err(())
                }
            }
            // c[impl exphist.unimplemented] - exponential histograms are rejected as input
            MetricData::ExponentialHistogram(_) => Err(()),
        }
    }
    match metric {
        AggregatedMetrics::F64(metric_data) => get_metric_data_type(metric_data),
        AggregatedMetrics::U64(metric_data) => get_metric_data_type(metric_data),
        AggregatedMetrics::I64(metric_data) => get_metric_data_type(metric_data),
    }
}

/// Write the current metric's metadata. Make sure to call [`extract_type_unit_and_name`] first.
// om[impl metadata.order] - TYPE is written first, then UNIT, then HELP
// om[impl metadata.unique] - at most one TYPE/UNIT/HELP line per family
#[inline]
fn write_header<U: uWrite>(ctx: &mut Context<'_, U>, description: &str) -> Result<(), U::Error> {
    let Context {
        f, name, unit, typ, ..
    } = ctx;
    // c[impl metadata.type]
    // om[impl text.lineending]
    for x in &["# TYPE ", name, " ", typ, "\n"] {
        f.write_str(x)?;
    }
    // om[impl metadata.unit-line]
    if let Some(unit) = unit {
        // om[impl text.lineending]
        for x in &["# UNIT ", name, " ", unit, "\n"] {
            f.write_str(x)?;
        }
    }
    // c[impl metadata.help-description]
    if !description.is_empty() {
        f.write_str("# HELP ")?;
        f.write_str(name)?;
        f.write_str(" ")?;
        write_escaped(f, description)?;
        // om[impl text.lineending]
        f.write_char('\n')?;
    }
    Ok(())
}

/// Write a `otel_scope` metric of type info for all scopes in `metrics`
/// according to the [spec](https://github.com/open-telemetry/opentelemetry-specification/blob/v1.45.0/specification/compatibility/prometheus_and_openmetrics.md#instrumentation-scope-1).
#[cfg(feature = "otel_scope_info")]
fn write_otel_scope_info<U: uWrite>(
    f: &mut U,
    metrics: &'_ Vec<&ScopeMetrics>,
) -> Result<(), U::Error> {
    // c[impl scope.info]
    f.write_str("# TYPE otel_scope info\n")?;

    for scope in metrics {
        // c[impl scope.name-version]
        let otel_attrs = make_scope_name_attrs(scope.scope().name(), scope.scope().version());
        f.write_str("otel_scope_info{")?;
        // c[impl scope.attribute-labels]
        write_attrs(f, otel_attrs.iter().chain(scope.scope().attributes()))?;
        // om[impl info.value]
        f.write_str("} 1\n")?;
    }
    Ok(())
}

/// Write all data points for this metric
fn write_values<U: uWrite>(
    ctx: &mut Context<'_, U>,
    metric: &AggregatedMetrics,
) -> Result<(), U::Error> {
    match metric {
        AggregatedMetrics::F64(metric_data) => {
            match metric_data {
                MetricData::Gauge(gauge) => write_gauge(ctx, gauge),
                MetricData::Sum(sum) => write_counter(ctx, sum),
                MetricData::Histogram(histogram) => write_histogram(ctx, histogram),
                _ => unimplemented!("only gauge/sum/histogram metrics should be constructible"),
                // See https://github.com/open-telemetry/opentelemetry-specification/blob/v1.45.0/specification/compatibility/prometheus_and_openmetrics.md#exponential-histograms
                // for exponential histograms
            }
        }
        AggregatedMetrics::U64(metric_data) => match metric_data {
            MetricData::Gauge(gauge) => write_gauge(ctx, gauge),
            MetricData::Sum(sum) => write_counter(ctx, sum),
            MetricData::Histogram(histogram) => write_histogram(ctx, histogram),
            _ => unimplemented!("only gauge/sum/histogram metrics should be constructible"),
        },
        AggregatedMetrics::I64(metric_data) => match metric_data {
            MetricData::Gauge(gauge) => write_gauge(ctx, gauge),
            MetricData::Sum(sum) => write_counter(ctx, sum),
            MetricData::Histogram(_) => {
                unimplemented!("signed histograms should not be constructible")
            }
            _ => unimplemented!("only gauge/sum/histogram metrics should be constructible"),
        },
    }
}

fn write_histogram<T: Numeric + Copy, U: uWrite>(
    ctx: &mut Context<'_, U>,
    histogram: &Histogram<T>,
) -> Result<(), U::Error> {
    // c[impl scope.labels-on-points]
    let scope_name_attrs = make_scope_name_attrs(ctx.scope_name, ctx.scope_version);
    let ts = to_timestamp(histogram.time());
    let created = to_timestamp(histogram.start_time());
    ctx.attr_buffer.clear();
    let attrs = &mut ctx.attr_buffer;
    let Ok(()) = write_attrs(attrs, scope_name_attrs.iter());
    // c[impl histogram.created]
    // FIXME: _created should be once per label set
    uwriteln!(
        ctx.f,
        "{}_created{{{}}} {} {}"
        ctx.name,
        attrs,
        created,
        ts,
    )?;
    assert_eq!(
        histogram.temporality(),
        Temporality::Cumulative,
        "Only cumulative Histograms are supported"
    );

    let mut points: Vec<_> = histogram.data_points().collect();
    points.sort_by_cached_key(|p| hash_attrs(p.attributes()));

    // om[impl metric.nointerleave] - each point's LabelSet is written contiguously
    for point in points {
        attrs.clear();
        let Ok(()) = write_attrs(attrs, point.attributes().chain(scope_name_attrs.iter()));

        // om[impl metricpoint.nointerleave] - all value samples of one MetricPoint are written contiguously
        // c[impl histogram.count]
        uwriteln!(
            ctx.f,
            "{}_count{{{}}} {} {}",
            ctx.name,
            attrs,
            point.count().fast_display(),
            ts
        )?;
        if T::is_unsigned() || point.min().as_ref().is_some_and(T::is_nonnegative) {
            // c[impl histogram.sum] - {name}_sum only if the sum is positive and monotonic
            // TODO: monitor if opentelmetry-sdk introduces positive f64 histograms?
            uwriteln!(
                ctx.f,
                "{}_sum{{{}}} {} {}",
                ctx.name,
                attrs,
                point.sum().fast_display(),
                ts,
            )?;
        }

        #[cfg(feature = "experimental-histogram-min-max")]
        {
            // Non-compliant but useful
            // TODO: Expose as a separate gauge?
            if let Some(min) = point.min() {
                uwriteln!(
                    ctx.f,
                    "{}_min{{{}}} {} {}",
                    ctx.name,
                    attrs,
                    min.fast_display(),
                    ts,
                )?;
            }
            if let Some(max) = point.max() {
                uwriteln!(
                    ctx.f,
                    "{}_max{{{}}} {} {}",
                    ctx.name,
                    attrs,
                    max.fast_display(),
                    ts,
                )?;
            }
        }

        if !attrs.is_empty() {
            attrs.push(',');
        }
        let mut cumulative_count = 0;
        // c[impl histogram.bucket]
        for (bound, count) in std::iter::zip(point.bounds(), point.bucket_counts()) {
            // c[impl histogram.bucket.cumulative]
            cumulative_count += count;
            // c[impl histogram.bucket.le]
            uwriteln!(
                // Not using write! here is a ~19% speedup
                ctx.f,
                "{}_bucket{{{}le=\"{}\"}} {} {}"
                ctx.name,
                attrs,
                bound.fast_display(),
                cumulative_count.fast_display(),
                ts,
            )?;
            // writeln!(
            //     f,
            //     "{name}_bucket{{{attrs}le=\"{bound}\"}} {count} {ts}",
            //     bound = bound.fast_display(),
            //     count = cumulative_count.fast_display(),
            // )?;
        }
        // om[impl histogram.inf-bucket]
        // c[impl histogram.bucket.inf]
        uwriteln!(
            ctx.f,
            "{}_bucket{{{}le=\"+Inf\"}} {} {}",
            ctx.name,
            attrs,
            point.count().fast_display(),
            ts,
        )?;
    }
    Ok(())
}

fn write_counter<T: Numeric + Copy, U: uWrite>(
    ctx: &mut Context<'_, U>,
    sum: &Sum<T>,
) -> Result<(), U::Error> {
    let attrs = &mut ctx.attr_buffer;
    // c[impl scope.labels-on-points]
    let scope_name_attrs = make_scope_name_attrs(ctx.scope_name, ctx.scope_version);
    assert_eq!(
        sum.temporality(),
        opentelemetry_sdk::metrics::Temporality::Cumulative,
        "Only cumulative sums are supported"
    );

    let mut points: Vec<_> = sum.data_points().collect();
    points.sort_by_cached_key(|p| hash_attrs(p.attributes()));

    let ts = to_timestamp(sum.time());

    // c[impl sum.total-suffix]
    if sum.is_monotonic() && !ctx.name.ends_with("_total") {
        // TODO: config option not to add _total
        for point in points {
            attrs.clear();
            let Ok(()) = write_attrs(attrs, point.attributes().chain(scope_name_attrs.iter()));
            uwriteln!(
                ctx.f,
                "{}_total{{{}}} {} {}",
                ctx.name,
                attrs,
                point.value().fast_display(),
                ts,
            )?;
        }
    } else {
        for point in points {
            attrs.clear();
            let Ok(()) = write_attrs(attrs, point.attributes().chain(scope_name_attrs.iter()));
            uwriteln!(
                ctx.f,
                "{}{{{}}} {} {}",
                ctx.name,
                attrs,
                point.value().fast_display(),
                ts,
            )?;
        }
    }
    Ok(())
}

fn write_gauge<T: Numeric + Copy, U: uWrite>(
    ctx: &mut Context<'_, U>,
    gauge: &Gauge<T>,
) -> Result<(), U::Error> {
    let attrs = &mut ctx.attr_buffer;
    // c[impl scope.labels-on-points]
    let scope_name_attrs = make_scope_name_attrs(ctx.scope_name, ctx.scope_version);
    let ts = to_timestamp(gauge.time());
    let mut points: Vec<_> = gauge.data_points().collect();
    points.sort_by_cached_key(|p| hash_attrs(p.attributes()));
    for point in points {
        attrs.clear();
        let Ok(()) = write_attrs(attrs, point.attributes().chain(scope_name_attrs.iter()));
        uwriteln!(
            ctx.f,
            "{}{{{}}} {} {}",
            ctx.name,
            attrs,
            point.value().fast_display(),
            ts,
        )?;
    }
    Ok(())
}

/// Makes an `otel_scope_name` attribute with the specified `scope_name` if the `otel_scope_info` feature is active.
// c[impl scope.labels-on-points]
#[inline]
fn make_scope_name_attrs(scope_name: &str, scope_version: Option<&str>) -> Vec<KeyValue> {
    // TODO: Get rid of the to_owned here, by not going through KeyValue
    if cfg!(feature = "otel_scope_info") {
        Some(KeyValue::new("otel_scope_name", scope_name.to_owned()))
            .into_iter()
            .chain(scope_version.map(|v| KeyValue::new("otel_scope_version", v.to_owned())))
            .collect()
    } else {
        Vec::new()
    }
}

/// Write the attribute string for attrs. Does not write curly braces.
fn write_attrs<'a, I: Iterator<Item = &'a KeyValue>, U: uWrite>(
    f: &mut U,
    attrs: I,
) -> Result<(), U::Error> {
    write_attrs_tuple(f, attrs.map(|kv| (&kv.key, &kv.value)))
}

fn write_attrs_tuple<'a, I: Iterator<Item = (&'a Key, &'a Value)>, U: uWrite>(
    f: &mut U,
    attrs: I,
) -> Result<(), U::Error> {
    // c[impl mattrs.to-labels] - attribute keys/values are emitted as label name/value pairs
    let mut first = true;

    let mut attrs: Vec<_> = attrs.collect();
    attrs.sort_unstable_by_key(|attr| attr.0);

    for attr in attrs {
        if !first {
            f.write_char(',')?;
        }
        write_sanitized_name(f, attr.0.as_str(), NameKind::AttributeLabel)?;
        f.write_str("=\"")?;
        // c[impl mattrs.type-conversion] - non-string values are stringified
        // c[impl attrs.stringify] - go through opentelemetry-sdk's `as_str`, TODO: implement our own to avoid allocating
        write_escaped(f, &attr.1.as_str())?;
        f.write_char('"')?;
        first = false;
    }
    Ok(())
}

/// Calculates a hash of the [`KeyValue`] pairs which is invariant under reordering of the [`KeyValue`]s within the [`Iterator`].
fn hash_attrs<'a, I: Iterator<Item = &'a KeyValue>>(attrs: I) -> u64 {
    let mut hash = 0;
    for kv in attrs {
        let mut hasher = DefaultHasher::default();
        hasher.write(kv.key.as_str().as_bytes());
        // TODO: replace `as_str` to avoid allocations
        hasher.write(kv.value.as_str().as_bytes());
        hash ^= hasher.finish(); // XOR to be order-invariant
    }
    hash
}

/// Writes to `f` the contents of `value` as an escaped string. Does not put quotes around the value.
/// The chars to escape are `\`, `"` and `\n`.
// om[impl escaping.chars]
fn write_escaped<U: uWrite>(f: &mut U, value: &str) -> Result<(), U::Error> {
    #[inline]
    fn next_escape_char(bytes: &[u8]) -> Option<usize> {
        #[cfg(feature = "fast")]
        return memchr::memchr3(b'\\', b'"', b'\n', bytes);
        #[cfg(not(feature = "fast"))]
        bytes
            .iter()
            .position(|&byte| byte == b'\\' || byte == b'"' || byte == b'\n')
    }

    let mut bytes = value.as_bytes();

    while let Some(next_escape) = next_escape_char(bytes) {
        let (head, tail) = bytes.split_at(next_escape);
        f.write_str(str::from_utf8(head).expect("escapable chars should be on a char boundary"))?;
        match tail[0] {
            b'\\' => f.write_str("\\\\"),
            b'"' => f.write_str("\\\""),
            b'\n' => f.write_str("\\n"),
            _ => unreachable!("next_escape_char should find one of the 3 escapable chars"),
        }?;
        bytes = &tail[1..];
    }
    // om[impl strings.utf8] - strings are written as valid UTF-8
    f.write_str(str::from_utf8(bytes).expect("escaped string should be valid utf-8"))
}

#[derive(Debug, PartialEq, Eq)]
enum NameKind {
    Metric,
    AttributeLabel,
}

/// Write `name` as an OpenMetrics metrics name, replacing any illegal characters with underscore according to the
/// [spec](https://github.com/open-telemetry/opentelemetry-specification/blob/v1.45.0/specification/compatibility/prometheus_and_openmetrics.md#metric-metadata-1).
// c[impl metadata.name-sanitize]
// c[impl mattrs.key-sanitize]
// om[related labels.reserved] - a leading illegal character is sanitized to
// a `_` prefix (e.g. `1.label` -> `_1_label`), producing an underscore-leading
// label name that technically violates om[labels.reserved]; accepted deviation,
// matching the opentelemetry-prometheus reference impl.
fn write_sanitized_name<U: uWrite>(f: &mut U, name: &str, kind: NameKind) -> Result<(), U::Error> {
    // Multiple consecutive `_` characters MUST be replaced with a single `_` character
    let mut previous_was_underscore = false;
    // The name must not start with a digit
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        f.write_char('_')?;
        previous_was_underscore = true;
    }
    for c in name.chars() {
        // Allowed characters are `a-z A-Z 0-9 : _`, except for ':' in labels
        // Invalid characters in the metric name MUST be replaced with the `_` character.
        if c.is_ascii_alphanumeric() || (c == ':' && kind == NameKind::Metric) {
            f.write_char(c)?;
            previous_was_underscore = false;
        } else {
            if !previous_was_underscore {
                f.write_char('_')?;
            }
            previous_was_underscore = true;
        }
    }
    Ok(())
}

/// Get a [`Display`] implementation which shows [`SystemTime`] as a unix timestamp in float seconds.
// om[impl timestamp.unix]
fn to_timestamp(time: SystemTime) -> impl uDisplay {
    let ts = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs_f64();
    ts.fast_display()
}
