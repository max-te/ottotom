use std::borrow::Cow;
use std::fmt::Write;
use std::hash::{DefaultHasher, Hasher};
use std::time::SystemTime;

use crate::format::Numeric;
use opentelemetry::{Key, KeyValue, SpanId, TraceId, Value};
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, Exemplar, Gauge, Histogram, MetricData, ResourceMetrics, Sum,
};
use opentelemetry_sdk::metrics::data::{Metric, ScopeMetrics};
use ufmt::{uDisplay, uWrite, uwrite, uwriteln};
use unit::get_unit_suffixes;

#[cfg(test)]
mod tests;
mod unit;

/// The mime type of the text produced by this metrics formatter.
// om[impl text.contenttype]
pub const MIME_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// Configuration for the OpenMetrics conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    otel_scope_info: bool,
    histogram_min_max: bool,
}

impl Config {
    /// Returns a builder for the conversion configuration.
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Preserve the historical default: the `otel_scope_info` cargo
            // feature was enabled by default.
            otel_scope_info: true,
            // The experimental min/max output is off by default; it can only be
            // enabled through the builder's `experimental`-gated setter.
            histogram_min_max: false,
        }
    }
}

/// Builder for [`Config`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConfigBuilder {
    config: Config,
}

impl ConfigBuilder {
    /// Sets whether to write `target_info` and `otel_scope_info` info metrics
    /// and add `otel_scope_name`/`otel_scope_version` labels on every point.
    pub fn otel_scope_info(mut self, value: bool) -> Self {
        self.config.otel_scope_info = value;
        self
    }

    /// Sets whether to emit non-compliant `_min`/`_max` samples for histograms.
    ///
    /// Available only when the `experimental` feature is enabled; the crate's
    /// own tests always see it.
    #[cfg(any(test, feature = "experimental"))]
    pub fn histogram_min_max(mut self, value: bool) -> Self {
        self.config.histogram_min_max = value;
        self
    }

    /// Builds the [`Config`].
    pub fn build(self) -> Config {
        self.config
    }
}

/// Trait to write the metrics data in OpenMetrics text format.
pub trait WriteOpenMetrics {
    /// Writes the metrics into `f` in OpenMetrics text format.
    fn write_as_openmetrics(&self, f: &mut impl Write) -> std::fmt::Result {
        self.write_as_openmetrics_with_config(f, Config::default())
    }
    /// Writes the metrics into `f` in OpenMetrics text format, honoring `config`.
    fn write_as_openmetrics_with_config(
        &self,
        f: &mut impl Write,
        config: Config,
    ) -> std::fmt::Result;
    /// Creates and returns a [String] of the metrics data in OpenMetrics text format.
    /// om[impl text.utf8] - output is always a valid UTF-8 String
    fn to_openmetrics_string(&self) -> Result<String, std::fmt::Error> {
        self.to_openmetrics_string_with_config(Config::default())
    }
    /// Creates and returns a [String] of the metrics data in OpenMetrics text
    /// format, honoring `config`.
    fn to_openmetrics_string_with_config(&self, config: Config) -> Result<String, std::fmt::Error> {
        let mut out = String::new();
        self.write_as_openmetrics_with_config(&mut out, config)?;
        Ok(out)
    }
}

/// Serialization context for common variables needed during conversion.
struct Context<'f, W: uWrite> {
    /// the output [Write] reference
    f: W,
    /// the conversion configuration
    config: Config,
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
    #[cfg(test)]
    fn with_output(f: &'f mut W) -> Self {
        Self::with_config(f, Config::default())
    }

    fn with_config(f: &'f mut W, config: Config) -> Self {
        Context {
            f: WriteAsUWrite(f),
            config,
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
    fn write_as_openmetrics_with_config(
        &self,
        f: &mut impl Write,
        config: Config,
    ) -> std::fmt::Result {
        let mut ctx = Context::with_config(f, config);

        if ctx.config.otel_scope_info {
            write_target_info(&mut ctx.f, self.resource())?;
        }

        let mut scopes: Vec<&ScopeMetrics> = self.scope_metrics().collect();
        scopes.sort_unstable_by_key(|s| s.scope().name());

        if ctx.config.otel_scope_info {
            // c[impl scope.config-disable] - the config struct is the configuration switch
            write_otel_scope_info(&mut ctx.f, &scopes, &ctx.config)?;
        }

        for scope in scopes {
            if ctx.config.otel_scope_info {
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
    // The family name must not end in `_total`: `write_counter` builds the
    // samples by appending `_total`/`_created` to it. A metric name that already
    // ends in `_total` is stripped here and re-appended by `write_counter`, so
    // the emitted sample name is unchanged.
    // om[related counter.suffix]
    // c[related sum.total-suffix]
    if ctx.typ == "counter" && ctx.name.ends_with("_total") {
        ctx.name.truncate(ctx.name.len() - "_total".len());
    }
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
fn write_otel_scope_info<U: uWrite>(
    f: &mut U,
    metrics: &'_ Vec<&ScopeMetrics>,
    config: &Config,
) -> Result<(), U::Error> {
    // c[impl scope.info]
    f.write_str("# TYPE otel_scope info\n")?;

    for scope in metrics {
        // c[impl scope.name-version]
        let otel_attrs =
            make_scope_name_attrs(config, scope.scope().name(), scope.scope().version());
        f.write_str("otel_scope_info{")?;
        // c[impl scope.attribute-labels]
        write_attrs(f, otel_attrs.iter().chain(scope.scope().attributes()))?;
        // om[impl info.value]
        f.write_str("} 1\n")?;
    }
    Ok(())
}

/// Write all data points for this metric
// c[impl resource.attrs] - resource attributes are dropped: labels on metric
// families carry only point and scope attributes, never resource attributes
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
    let scope_name_attrs = make_scope_name_attrs(&ctx.config, ctx.scope_name, ctx.scope_version);
    let ts = to_timestamp(histogram.time());
    let created = to_timestamp(histogram.start_time());
    let attrs = &mut ctx.attr_buffer;
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

        // c[impl histogram.created]
        uwriteln!(
            ctx.f,
            "{}_created{{{}}} {} {}"
            ctx.name,
            attrs,
            created,
            ts,
        )?;

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

        if ctx.config.histogram_min_max {
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
        let bounds: Vec<_> = point.bounds().collect();
        for (i, (bound, count)) in std::iter::zip(&bounds, point.bucket_counts()).enumerate() {
            // c[impl histogram.bucket.cumulative]
            cumulative_count += count;
            // c[impl histogram.bucket.le]
            uwrite!(
                // Not using write! here is a ~19% speedup
                ctx.f,
                "{}_bucket{{{}le=\"{}\"}} {} {}",
                ctx.name,
                attrs,
                bound.fast_display(),
                cumulative_count.fast_display(),
                ts,
            )?;
            // om[impl exemplars.bucket-attachment] - attach the latest exemplar
            // whose value falls within this bucket; a bucket holds at most one.
            // c[impl exemplar.bucket-single]
            // c[impl histogram.bucket.exemplar] - a single exemplar per `le`
            // bucket, attached to no other `le`-labelled point
            let lower = if i > 0 {
                bounds[i - 1]
            } else {
                f64::NEG_INFINITY
            };
            write_exemplar(
                &mut ctx.f,
                point
                    .exemplars()
                    .filter(|e| e.value.to_f64() > lower && e.value.to_f64() <= *bound),
            )?;
            ctx.f.write_char('\n')?;
        }
        // om[impl histogram.inf-bucket]
        // c[impl histogram.bucket.inf]
        uwrite!(
            ctx.f,
            "{}_bucket{{{}le=\"+Inf\"}} {} {}",
            ctx.name,
            attrs,
            point.count().fast_display(),
            ts,
        )?;
        // Exemplars above the last finite bound belong to the +Inf bucket.
        if let Some(&last) = bounds.last() {
            write_exemplar(
                &mut ctx.f,
                point.exemplars().filter(|e| e.value.to_f64() > last),
            )?;
        }
        ctx.f.write_char('\n')?;
    }
    Ok(())
}

fn write_counter<T: Numeric + Copy, U: uWrite>(
    ctx: &mut Context<'_, U>,
    sum: &Sum<T>,
) -> Result<(), U::Error> {
    let attrs = &mut ctx.attr_buffer;
    // c[impl scope.labels-on-points]
    let scope_name_attrs = make_scope_name_attrs(&ctx.config, ctx.scope_name, ctx.scope_version);
    assert_eq!(
        sum.temporality(),
        opentelemetry_sdk::metrics::Temporality::Cumulative,
        "Only cumulative sums are supported"
    );

    let mut points: Vec<_> = sum.data_points().collect();
    points.sort_by_cached_key(|p| hash_attrs(p.attributes()));

    let ts = to_timestamp(sum.time());
    let created = to_timestamp(sum.start_time());

    if sum.is_monotonic() {
        for point in points {
            attrs.clear();
            let Ok(()) = write_attrs(attrs, point.attributes().chain(scope_name_attrs.iter()));
            // c[impl sum.created]
            // om[impl counter.suffix]
            uwriteln!(
                ctx.f,
                "{}_created{{{}}} {} {}",
                ctx.name,
                attrs,
                created,
                ts
            )?;
            // c[impl sum.total-suffix] - the family name is bare (see
            // `extract_type_unit_and_name`), so the `_total` suffix is always
            // appended to the value sample.
            uwrite!(
                ctx.f,
                "{}_total{{{}}} {} {}",
                ctx.name,
                attrs,
                point.value().fast_display(),
                ts,
            )?;
            // c[impl exemplar.types] - exemplars on monotonic sums are converted
            write_exemplar(&mut ctx.f, point.exemplars())?;
            ctx.f.write_char('\n')?;
        }
    } else {
        for point in points {
            attrs.clear();
            let Ok(()) = write_attrs(attrs, point.attributes().chain(scope_name_attrs.iter()));
            // c[impl exemplar.types] - exemplars on non-monotonic sums (gauges)
            // are dropped
            uwrite!(
                ctx.f,
                "{}{{{}}} {} {}",
                ctx.name,
                attrs,
                point.value().fast_display(),
                ts,
            )?;
            ctx.f.write_char('\n')?;
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
    let scope_name_attrs = make_scope_name_attrs(&ctx.config, ctx.scope_name, ctx.scope_version);
    let ts = to_timestamp(gauge.time());
    let mut points: Vec<_> = gauge.data_points().collect();
    points.sort_by_cached_key(|p| hash_attrs(p.attributes()));
    for point in points {
        attrs.clear();
        let Ok(()) = write_attrs(attrs, point.attributes().chain(scope_name_attrs.iter()));
        // c[impl exemplar.types] - exemplars on gauges are dropped
        uwrite!(
            ctx.f,
            "{}{{{}}} {} {}",
            ctx.name,
            attrs,
            point.value().fast_display(),
            ts,
        )?;
        ctx.f.write_char('\n')?;
    }
    Ok(())
}

/// Makes an `otel_scope_name` attribute with the specified `scope_name` if
/// the config enables `otel_scope_info`.
// c[impl scope.labels-on-points]
#[inline]
fn make_scope_name_attrs(
    config: &Config,
    scope_name: &str,
    scope_version: Option<&str>,
) -> Vec<KeyValue> {
    // TODO: Get rid of the to_owned here, by not going through KeyValue
    if config.otel_scope_info {
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

fn write_exemplar<'a, T: Numeric + Copy + 'a, U: uWrite>(
    f: &mut U,
    exemplars: impl IntoIterator<Item = &'a Exemplar<T>>,
) -> Result<(), U::Error> {
    // Write at most one exemplar, preferring the most recent measurement.
    let Some(exemplar) = exemplars.into_iter().max_by_key(|e| e.time()) else {
        return Ok(());
    };
    // Build the exemplar's label set: the ids (only when a trace context was
    // active; the SDK zeroes them otherwise) plus the filtered attributes.
    // c[impl exemplar.trace-span-ids]
    let mut labels = Vec::new();
    let trace_id = *exemplar.trace_id();
    if trace_id != [0; 16] {
        labels.push(KeyValue::new(
            "trace_id",
            TraceId::from_bytes(trace_id).to_string(),
        ));
    }
    let span_id = *exemplar.span_id();
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
        exemplar.value.fast_display(),
        to_timestamp(exemplar.time()),
    )?;
    Ok(())
}
