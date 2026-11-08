use std::borrow::Cow;
use std::fmt::Write;
use std::time::SystemTime;

use crate::format::{Numeric, canonical_display};
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, Gauge, GaugeDataPoint, Histogram, HistogramDataPoint, MetricData,
    ResourceMetrics, Sum, SumDataPoint,
};
use opentelemetry_sdk::metrics::data::{Metric, ScopeMetrics};
use ufmt::{uDisplay, uWrite, uwrite, uwriteln};
use unit::get_unit_suffixes;

use exemplar::{write_bucket_exemplar, write_exemplar};
use label_sets::{LabeledPoint, drop_repeated_label_sets};
use labels::{
    LabelScratch, NameKind, hash_attrs, sanitizes_to, write_attrs_tuple, write_attrs_with,
    write_escaped, write_sanitized_name,
};

mod exemplar;
mod label_sets;
mod labels;
#[cfg(test)]
mod tests;
mod unit;

/// The mime type of the text produced by this metrics formatter.
// om[impl text.contenttype]
pub const MIME_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// Configuration for the OpenMetrics conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    scope_info_enabled: bool,
    target_info_enabled: bool,
    histogram_min_max: bool,
}

impl Config {
    /// Returns a builder for the conversion configuration.
    #[must_use]
    // The body already is `Default::default()`; the mutant is equivalent.
    #[cfg_attr(test, mutants::skip)]
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            scope_info_enabled: true,
            target_info_enabled: true,
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
    /// Sets whether to write `otel_scope_info` info metrics
    /// and add `otel_scope_name`/`otel_scope_version` labels on every point.
    /// This option is enabled by default.
    #[must_use]
    pub fn scope_info_enabled(mut self, value: bool) -> Self {
        self.config.scope_info_enabled = value;
        self
    }

    /// Sets whether to write `target_info` info metrics.
    /// This option is enabled by default.
    #[must_use]
    pub fn target_info_enabled(mut self, value: bool) -> Self {
        self.config.target_info_enabled = value;
        self
    }

    /// Sets whether to emit non-compliant `_min`/`_max` samples for histograms.
    /// This option is disabled by default.
    #[cfg(any(test, feature = "experimental"))]
    #[must_use]
    pub fn histogram_min_max(mut self, value: bool) -> Self {
        self.config.histogram_min_max = value;
        self
    }

    /// Builds the [`Config`].
    #[must_use]
    pub fn build(self) -> Config {
        self.config
    }
}

/// Trait to write the metrics data in OpenMetrics text format.
pub trait WriteOpenMetrics: crate::private::Sealed {
    /// Writes the metrics into `f` in OpenMetrics text format.
    ///
    /// # Errors
    ///
    /// Propagates whatever `f` reports; the conversion itself cannot fail.
    fn write_as_openmetrics(&self, f: &mut impl Write) -> std::fmt::Result {
        self.write_as_openmetrics_with_config(f, Config::default())
    }
    /// Writes the metrics into `f` in OpenMetrics text format, honoring `config`.
    ///
    /// # Errors
    ///
    /// Propagates whatever `f` reports; the conversion itself cannot fail.
    fn write_as_openmetrics_with_config(
        &self,
        f: &mut impl Write,
        config: Config,
    ) -> std::fmt::Result;
    /// Creates and returns a [String] of the metrics data in OpenMetrics text format.
    /// om[impl text.utf8] - output is always a valid UTF-8 String
    ///
    /// # Errors
    ///
    /// Only if writing into the returned [`String`] fails, which a `String`
    /// does not do.
    fn to_openmetrics_string(&self) -> Result<String, std::fmt::Error> {
        self.to_openmetrics_string_with_config(Config::default())
    }
    /// Creates and returns a [String] of the metrics data in OpenMetrics text
    /// format, honoring `config`.
    ///
    /// # Errors
    ///
    /// Only if writing into the returned [`String`] fails, which a `String`
    /// does not do.
    fn to_openmetrics_string_with_config(&self, config: Config) -> Result<String, std::fmt::Error> {
        let mut out = String::new();
        self.write_as_openmetrics_with_config(&mut out, config)?;
        Ok(out)
    }
}

/// Serialization context for common variables needed during conversion.
struct Context<W: uWrite> {
    /// the output [Write] reference
    f: W,
    /// the conversion configuration
    config: Config,
    /// the label sets of the current family's points, back to back
    attr_buffer: String,
    /// the current family's points, each with its label set in `attr_buffer`
    labeled_points: Vec<LabeledPoint>,
    /// scratch space for writing label sets, reused across points
    label_scratch: LabelScratch,
    /// the `le` labels of the last histogram point
    le_labels: LeLabels,
    /// the scope that tests write single metrics in
    #[cfg(test)]
    scope_name: &'static str,
    #[cfg(test)]
    scope_version: Option<&'static str>,
}

impl<'f, W: Write> Context<WriteAsUWrite<'f, W>> {
    #[cfg(test)]
    fn with_output(f: &'f mut W) -> Self {
        Self::with_config(f, Config::default())
    }

    fn with_config(f: &'f mut W, config: Config) -> Self {
        Context {
            f: WriteAsUWrite(f),
            config,
            attr_buffer: String::with_capacity(256),
            labeled_points: Vec::new(),
            label_scratch: LabelScratch::default(),
            le_labels: LeLabels::default(),
            #[cfg(test)]
            scope_name: "",
            #[cfg(test)]
            scope_version: None,
        }
    }
}

impl<U: uWrite> Context<U> {
    /// Starts writing the family `name` of type `typ`.
    fn family<'c>(
        &'c mut self,
        name: &'c str,
        typ: &'static str,
        unit: Option<&'c str>,
    ) -> FamilyContext<'c, U> {
        FamilyContext {
            ctx: self,
            name,
            typ,
            unit,
        }
    }
}

/// A [`Context`] while one family is written, with the family's metadata.
struct FamilyContext<'c, U: uWrite> {
    ctx: &'c mut Context<U>,
    /// the sanitized family name, which starts every sample name
    name: &'c str,
    /// the OpenMetrics metric type
    typ: &'static str,
    /// the unit, already appended to `name`
    unit: Option<&'c str>,
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

impl crate::private::Sealed for ResourceMetrics {}

impl WriteOpenMetrics for ResourceMetrics {
    fn write_as_openmetrics_with_config(
        &self,
        f: &mut impl Write,
        config: Config,
    ) -> std::fmt::Result {
        let mut ctx = Context::with_config(f, config);

        if ctx.config.target_info_enabled {
            write_target_info(&mut ctx.f, self.resource())?;
        }

        let mut scopes: Vec<&ScopeMetrics> = self.scope_metrics().collect();
        scopes.sort_by_cached_key(|s| {
            let scope = s.scope();
            (
                scope.name(),
                scope.version(),
                hash_attrs(scope.attributes()),
            )
        });

        if ctx.config.scope_info_enabled {
            // c[impl scope.config-disable] - the config struct is the configuration switch
            write_otel_scope_info(&mut ctx.f, &scopes, ctx.config)?;
        }

        let Plan { names, mut members } = Plan::new(&scopes);
        let family_name = |member: &Member<'_>| &names[member.family.0..member.family.1];
        // Each family as the range of its members; the plan is ordered by
        // family name, so a family's members are adjacent.
        let mut spans = Vec::new();
        let mut start = 0;
        while start < members.len() {
            let name = family_name(&members[start]);
            let len = members[start..]
                .iter()
                .take_while(|member| family_name(member) == name)
                .count();
            spans.push(start..start + len);
            start += len;
        }

        // The info families are written first, so their names are claimed first.
        let mut families: Vec<(&str, &'static str)> = Vec::with_capacity(spans.len() + 2);
        if ctx.config.target_info_enabled {
            families.push(("target", "info"));
        }
        if ctx.config.scope_info_enabled {
            families.push(("otel_scope", "info"));
        }
        let info_families = families.len();
        families.extend(
            spans
                .iter()
                .map(|span| (family_name(&members[span.start]), members[span.start].typ)),
        );
        let clashes = find_name_clashes(&families);

        // om[impl metricfamily.nointerleave] - each MetricFamily is fully written before the next
        for (span, clashes) in spans.into_iter().zip(&clashes[info_families..]) {
            let family = &mut members[span];
            let name = family_name(&family[0]);
            if *clashes {
                // c[impl metadata.drop-warn]
                warn!(
                    points = family
                        .iter()
                        .map(|member| point_count(member.metric.data()))
                        .sum::<usize>(),
                    "Dropping family {name}: one of its sample names is already written by another family"
                );
                continue;
            }

            // c[impl metadata.conflict-type] - the plan orders by type within a
            // family, so the first member's type wins
            let typ = family[0].typ;
            // c[impl metadata.conflict-unit-help] - the first UNIT and HELP are kept
            let unit = family
                .iter_mut()
                .filter(|member| member.typ == typ)
                .find_map(|member| member.unit.take());
            let description = family
                .iter()
                .filter(|member| member.typ == typ)
                .map(|member| member.metric.description())
                .find(|description| !description.is_empty())
                .unwrap_or_default();
            // c[impl metadata.conflict-keep-points] - every member of the family's type is written
            let mut written = Vec::with_capacity(family.len());
            for member in family.iter() {
                if member.typ != typ {
                    // c[impl metadata.drop-warn]
                    warn!(
                        points = point_count(member.metric.data()),
                        "Dropping metric {:?}: family {name} is already of type {typ}, not {}",
                        member.metric.name(),
                        member.typ
                    );
                    continue;
                }
                let scope = (member.scope.name(), member.scope.version());
                written.push((scope, member.metric.data()));
            }
            let mut family_ctx = ctx.family(name, typ, unit.as_deref());
            write_points(&mut family_ctx, Some(description), &written)?;
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
    write_attrs_tuple(f, &mut LabelScratch::default(), resource.iter())?;
    // om[impl info.value]
    f.write_str("} 1\n")?;
    Ok(())
}

/// One metric, placed in the family it is written to.
struct Member<'m> {
    /// byte range of the family name in [`Plan::names`]
    family: (usize, usize),
    typ: &'static str,
    unit: Option<Cow<'static, str>>,
    scope: &'m InstrumentationScope,
    /// order-independent hash of the scope attributes, to order scopes that share name and version
    scope_hash: u64,
    metric: &'m Metric,
}

/// The metrics of one conversion, in the order their families are written.
struct Plan<'m> {
    /// all family names, back to back
    names: String,
    members: Vec<Member<'m>>,
}

impl<'m> Plan<'m> {
    /// Places every supported metric of `scopes` in its family and orders the
    /// result by family name. Unsupported metrics are dropped with a warning.
    fn new(scopes: &[&'m ScopeMetrics]) -> Self {
        let mut names = String::new();
        let mut name = String::with_capacity(64);
        let mut members = Vec::new();
        for scope_metrics in scopes {
            let scope = scope_metrics.scope();
            let scope_hash = hash_attrs(scope.attributes());
            for metric in scope_metrics.metrics() {
                if let Some((typ, unit)) = extract_type_unit_and_name(&mut name, metric) {
                    let start = names.len();
                    names.push_str(&name);
                    members.push(Member {
                        family: (start, names.len()),
                        typ,
                        unit,
                        scope,
                        scope_hash,
                        metric,
                    });
                } else {
                    // c[impl metadata.drop-warn]
                    warn!(
                        points = point_count(metric.data()),
                        "Unsupported metric type {metric:?}"
                    );
                }
            }
        }
        // Deterministic output: the order depends on what is written, not on
        // the order in which the SDK reports scopes and metrics.
        members.sort_by(|a, b| {
            names[a.family.0..a.family.1]
                .cmp(&names[b.family.0..b.family.1])
                .then_with(|| a.typ.cmp(b.typ))
                .then_with(|| a.scope.name().cmp(b.scope.name()))
                .then_with(|| a.scope.version().cmp(&b.scope.version()))
                .then_with(|| a.scope_hash.cmp(&b.scope_hash))
                .then_with(|| a.metric.name().cmp(b.metric.name()))
                .then_with(|| a.metric.unit().cmp(b.metric.unit()))
                .then_with(|| data_kind(a.metric.data()).cmp(&data_kind(b.metric.data())))
                .then_with(|| a.metric.description().cmp(b.metric.description()))
        });
        Plan { names, members }
    }
}

/// The sample names a family of type `typ` writes, as suffixes of its name.
fn sample_suffixes(typ: &str) -> &'static [&'static str] {
    match typ {
        "counter" => &["_total", "_created"],
        "histogram" => &["_bucket", "_count", "_sum", "_created"],
        "info" => &["_info"],
        _ => &[""],
    }
}

/// For each family in `families` (name and type, in writing order), whether it
/// must be dropped because a family written before it already uses its family
/// name or one of its sample names.
// om[impl metricfamily.name-clash]
fn find_name_clashes(families: &[(&str, &'static str)]) -> Vec<bool> {
    /// A name a family writes: its family name (`is_sample == false`, empty
    /// suffix) or one of its sample names. The name is the family name
    /// followed by `suffix`, compared without being built.
    struct Claim {
        is_sample: bool,
        family: usize,
        suffix: &'static str,
        /// position before sorting, to look up `run_of` per family
        position: usize,
    }

    let mut claims = Vec::new();
    let mut first_claim = Vec::with_capacity(families.len() + 1);
    for (family, &(_, typ)) in families.iter().enumerate() {
        first_claim.push(claims.len());
        let names =
            std::iter::once((false, "")).chain(sample_suffixes(typ).iter().map(|s| (true, *s)));
        for (is_sample, suffix) in names {
            let position = claims.len();
            claims.push(Claim {
                is_sample,
                family,
                suffix,
                position,
            });
        }
    }
    first_claim.push(claims.len());

    let text = |claim: &Claim| families[claim.family].0.bytes().chain(claim.suffix.bytes());
    claims.sort_by(|a, b| {
        a.is_sample
            .cmp(&b.is_sample)
            .then_with(|| text(a).cmp(text(b)))
    });
    // Claims of one name share a run id.
    let mut run_of = vec![0; claims.len()];
    let mut run = 0;
    for (i, claim) in claims.iter().enumerate() {
        if i > 0
            && (claims[i - 1].is_sample != claim.is_sample || !text(&claims[i - 1]).eq(text(claim)))
        {
            run += 1;
        }
        run_of[claim.position] = run;
    }

    // A family is written only if none of its names is taken; then it takes them all.
    let mut taken = vec![false; run + 1];
    (0..families.len())
        .map(|family| {
            let runs = &run_of[first_claim[family]..first_claim[family + 1]];
            let clashes = runs.iter().any(|&run| taken[run]);
            if !clashes {
                for &run in runs {
                    taken[run] = true;
                }
            }
            clashes
        })
        .collect()
}

/// The number of data points in `data`, which a dropping warning reports.
fn point_count(data: &AggregatedMetrics) -> usize {
    fn count<T>(data: &MetricData<T>) -> usize {
        match data {
            MetricData::Gauge(gauge) => gauge.data_points().count(),
            MetricData::Sum(sum) => sum.data_points().count(),
            MetricData::Histogram(histogram) => histogram.data_points().count(),
            MetricData::ExponentialHistogram(histogram) => histogram.data_points().count(),
        }
    }
    match data {
        AggregatedMetrics::F64(data) => count(data),
        AggregatedMetrics::U64(data) => count(data),
        AggregatedMetrics::I64(data) => count(data),
    }
}

/// Orders metrics of one name that differ only in their number type or in
/// their aggregation, such as a gauge and an up-down counter.
fn data_kind(data: &AggregatedMetrics) -> (u8, u8) {
    fn aggregation<T>(data: &MetricData<T>) -> u8 {
        match data {
            MetricData::Gauge(_) => 0,
            MetricData::Sum(_) => 1,
            MetricData::Histogram(_) => 2,
            MetricData::ExponentialHistogram(_) => 3,
        }
    }
    match data {
        AggregatedMetrics::F64(data) => (0, aggregation(data)),
        AggregatedMetrics::U64(data) => (1, aggregation(data)),
        AggregatedMetrics::I64(data) => (2, aggregation(data)),
    }
}

/// Writes the family name of `metric` into `name` and returns its type and
/// unit, or `None` if the metric is not supported.
fn extract_type_unit_and_name(
    name: &mut String,
    metric: &Metric,
) -> Option<(&'static str, Option<Cow<'static, str>>)> {
    let typ = get_type(metric.data()).ok()?;
    let unit = get_unit_suffixes(metric.unit());

    name.clear();
    let Ok(()) = write_sanitized_name(name, metric.name(), NameKind::Metric);
    // The family name must not end in `_total`: `write_counter` builds the
    // samples by appending `_total`/`_created` to it. A metric name that already
    // ends in `_total` is stripped here and re-appended by `write_counter`, so
    // the emitted sample name is unchanged.
    // om[related counter.suffix]
    // c[related sum.total-suffix]
    if typ == "counter" && name.ends_with("_total") {
        name.truncate(name.len() - "_total".len());
    }
    // om[impl metadata.unit-suffix]
    // c[impl metadata.unit-suffix]
    if let Some(ref unit) = unit
        && !name.ends_with(unit.as_ref())
    {
        name.push('_');
        name.push_str(unit);
    }

    Some((typ, unit))
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

/// Write the metadata of `family`.
// om[impl metadata.order] - TYPE is written first, then UNIT, then HELP
// om[impl metadata.unique] - at most one TYPE/UNIT/HELP line per family
#[inline]
fn write_header<U: uWrite>(
    family: &mut FamilyContext<'_, U>,
    description: &str,
) -> Result<(), U::Error> {
    let FamilyContext {
        ctx,
        name,
        typ,
        unit,
    } = family;
    let f = &mut ctx.f;
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
    config: Config,
) -> Result<(), U::Error> {
    // c[impl scope.info]
    f.write_str("# TYPE otel_scope info\n")?;

    let mut scratch = LabelScratch::default();
    for scope in metrics {
        // c[impl scope.name-version]
        let otel_attrs =
            make_scope_name_attrs(config, scope.scope().name(), scope.scope().version());
        f.write_str("otel_scope_info{")?;
        // c[impl scope.attribute-labels]
        write_attrs_with(
            f,
            &mut scratch,
            otel_attrs.iter().chain(scope.scope().attributes()),
        )?;
        // om[impl info.value]
        f.write_str("} 1\n")?;
    }
    Ok(())
}

/// Writes the points of one metric as a whole family, in the test scope.
#[cfg(test)]
fn write_in_test_scope<M: FamilyMember, U: uWrite>(
    family: &mut FamilyContext<'_, U>,
    metric: &M,
) -> Result<(), U::Error> {
    let scope = (family.ctx.scope_name, family.ctx.scope_version);
    write_points(family, None, &[(scope, metric)])
}

#[cfg(test)]
fn write_values<U: uWrite>(
    family: &mut FamilyContext<'_, U>,
    metric: &AggregatedMetrics,
) -> Result<(), U::Error> {
    write_in_test_scope(family, metric)
}

/// The name and version of an instrumentation scope.
type ScopeLabels<'s> = (&'s str, Option<&'s str>);

/// Writes the points of one family. `members` holds each metric of the family
/// with the scope it was recorded in.
///
/// Each metric's points are written in the order of their label sets. A point
/// whose label set another point of the family already has is dropped.
///
/// With `help`, the family's header is written first, with `help` as its HELP
/// text. A family whose points are all dropped then writes nothing at all.
// c[impl resource.attrs] - resource attributes are dropped: labels on metric
// families carry only point and scope attributes, never resource attributes
fn write_points<M: FamilyMember, U: uWrite>(
    family: &mut FamilyContext<'_, U>,
    help: Option<&str>,
    members: &[(ScopeLabels<'_>, &M)],
) -> Result<(), U::Error> {
    // Taken out of `family` while the members write, so their capacity is reused.
    let mut text = std::mem::take(&mut family.ctx.attr_buffer);
    let mut points = std::mem::take(&mut family.ctx.labeled_points);
    text.clear();
    points.clear();

    for (member, (scope, data)) in members.iter().enumerate() {
        data.render_labels(family, *scope, member, &mut text, &mut points);
    }
    drop_repeated_label_sets(family.name, &text, &mut points, members.len(), |member| {
        members[member].1.hash_points()
    });

    let written = write_kept_points(family, help, members, &points, &text);

    family.ctx.attr_buffer = text;
    family.ctx.labeled_points = points;
    written
}

/// Writes the header, if `help` is given, and then the kept `points`, grouped
/// by member.
fn write_kept_points<M: FamilyMember, U: uWrite>(
    family: &mut FamilyContext<'_, U>,
    help: Option<&str>,
    members: &[(ScopeLabels<'_>, &M)],
    points: &[LabeledPoint],
    text: &str,
) -> Result<(), U::Error> {
    if let Some(help) = help {
        // A header alone carries no data, and strict parsers reject an empty family.
        if points.is_empty() {
            return Ok(());
        }
        // om[impl metadata.unique] - one header per family, however many metrics it holds
        // c[impl metadata.no-duplicates] - the plan gives each metric name one family
        write_header(family, help)?;
    }
    // Every member writes, even one without points, so each checks its data.
    let mut rest = points;
    for (member, (_, data)) in members.iter().enumerate() {
        let (own, later) = rest.split_at(rest.partition_point(|p| p.member == member));
        rest = later;
        data.write_labeled(family, own, text)?;
    }
    Ok(())
}

/// Metric data that contributes points to a family.
trait FamilyMember {
    /// Renders the label set of each point onto `text` and records the point
    /// in `points`. A point that would shadow a reserved label is skipped.
    fn render_labels<U: uWrite>(
        &self,
        family: &mut FamilyContext<'_, U>,
        scope: ScopeLabels<'_>,
        member: usize,
        text: &mut String,
        points: &mut Vec<LabeledPoint>,
    );

    /// The [`hash_attrs`] of each point, in point order.
    fn hash_points(&self) -> Vec<u64>;

    /// Writes `points` of this metric, in the given order.
    fn write_labeled<U: uWrite>(
        &self,
        family: &mut FamilyContext<'_, U>,
        points: &[LabeledPoint],
        text: &str,
    ) -> Result<(), U::Error>;
}

/// The gauge, sum or histogram of one metric, whose points are written alike.
trait DataPoints {
    type Point;
    /// Whether the points are written as buckets, which reserve the label `le`.
    const HISTOGRAM: bool;

    fn points(&self) -> impl Iterator<Item = &Self::Point>;

    fn attributes(point: &Self::Point) -> impl Iterator<Item = &KeyValue>;

    /// Whether the value of `point` can be written. A point whose value would
    /// be invalid output is reported as dropped.
    fn keeps_value(&self, _metric: &str, _point: &Self::Point) -> bool {
        true
    }

    /// Writes the samples of `points`, each with its rendered label set.
    fn write_samples<'p, U: uWrite>(
        &'p self,
        family: &mut FamilyContext<'_, U>,
        points: impl Iterator<Item = (&'p Self::Point, &'p str)>,
    ) -> Result<(), U::Error>;
}

impl<D: DataPoints> FamilyMember for D {
    fn render_labels<U: uWrite>(
        &self,
        family: &mut FamilyContext<'_, U>,
        (scope_name, scope_version): ScopeLabels<'_>,
        member: usize,
        text: &mut String,
        points: &mut Vec<LabeledPoint>,
    ) {
        // c[impl scope.labels-on-points]
        let scope_name_attrs = make_scope_name_attrs(family.ctx.config, scope_name, scope_version);
        let reserved = reserved_labels(family.ctx.config, D::HISTOGRAM);
        let data_points = self.points();
        let count = data_points.size_hint().0;
        points.reserve(count);
        for (index, point) in data_points.enumerate() {
            if !keeps_reserved_labels(family.name, D::attributes(point), reserved)
                || !self.keeps_value(family.name, point)
            {
                continue;
            }
            let start = text.len();
            let Ok(()) = write_attrs_with(
                text,
                &mut family.ctx.label_scratch,
                D::attributes(point).chain(scope_name_attrs.iter()),
            );
            points.push(LabeledPoint {
                member,
                point: index,
                start,
                end: text.len(),
                key: 0,
            });
            if index == 0 {
                // Label sets of one metric tend to have similar lengths.
                // The headroom covers somewhat longer values.
                let estimate = text.len() - start + 8;
                text.reserve(estimate * count.saturating_sub(1));
            }
        }
    }

    fn hash_points(&self) -> Vec<u64> {
        self.points()
            .map(|point| hash_attrs(D::attributes(point)))
            .collect()
    }

    fn write_labeled<U: uWrite>(
        &self,
        family: &mut FamilyContext<'_, U>,
        points: &[LabeledPoint],
        text: &str,
    ) -> Result<(), U::Error> {
        let data_points: Vec<_> = self.points().collect();
        self.write_samples(
            family,
            points
                .iter()
                .map(|p| (data_points[p.point], &text[p.start..p.end])),
        )
    }
}

/// Evaluates `$body` with `$data` bound to the gauge, sum or histogram in `$metric`.
macro_rules! with_data_points {
    ($metric:expr, $data:ident => $body:expr) => {
        match $metric {
            AggregatedMetrics::F64(MetricData::Gauge($data)) => $body,
            AggregatedMetrics::F64(MetricData::Sum($data)) => $body,
            AggregatedMetrics::F64(MetricData::Histogram($data)) => $body,
            AggregatedMetrics::U64(MetricData::Gauge($data)) => $body,
            AggregatedMetrics::U64(MetricData::Sum($data)) => $body,
            AggregatedMetrics::U64(MetricData::Histogram($data)) => $body,
            AggregatedMetrics::I64(MetricData::Gauge($data)) => $body,
            AggregatedMetrics::I64(MetricData::Sum($data)) => $body,
            AggregatedMetrics::I64(MetricData::Histogram(_)) => {
                unimplemented!("signed histograms should not be constructible")
            }
            // See https://github.com/open-telemetry/opentelemetry-specification/blob/v1.45.0/specification/compatibility/prometheus_and_openmetrics.md#exponential-histograms
            // for exponential histograms
            _ => unimplemented!("only gauge/sum/histogram metrics should be constructible"),
        }
    };
}

impl FamilyMember for AggregatedMetrics {
    fn render_labels<U: uWrite>(
        &self,
        family: &mut FamilyContext<'_, U>,
        scope: ScopeLabels<'_>,
        member: usize,
        text: &mut String,
        points: &mut Vec<LabeledPoint>,
    ) {
        with_data_points!(self, data => data.render_labels(family, scope, member, text, points));
    }

    fn hash_points(&self) -> Vec<u64> {
        with_data_points!(self, data => data.hash_points())
    }

    fn write_labeled<U: uWrite>(
        &self,
        family: &mut FamilyContext<'_, U>,
        points: &[LabeledPoint],
        text: &str,
    ) -> Result<(), U::Error> {
        with_data_points!(self, data => data.write_labeled(family, points, text))
    }
}

#[cfg(test)]
fn write_histogram<T: Numeric + Copy, U: uWrite>(
    family: &mut FamilyContext<'_, U>,
    histogram: &Histogram<T>,
) -> Result<(), U::Error> {
    write_in_test_scope(family, histogram)
}

impl<T: Numeric + Copy> DataPoints for Histogram<T> {
    type Point = HistogramDataPoint<T>;
    const HISTOGRAM: bool = true;

    fn points(&self) -> impl Iterator<Item = &Self::Point> {
        self.data_points()
    }

    fn attributes(point: &Self::Point) -> impl Iterator<Item = &KeyValue> {
        point.attributes()
    }

    fn write_samples<'p, U: uWrite>(
        &'p self,
        family: &mut FamilyContext<'_, U>,
        points: impl Iterator<Item = (&'p Self::Point, &'p str)>,
    ) -> Result<(), U::Error> {
        write_histogram_samples(family, self, points)
    }
}

/// The bounds of the last histogram point, with their `le` label values. All
/// points of one histogram share their bounds, so the labels are rendered once.
#[derive(Default)]
struct LeLabels {
    bounds: Vec<f64>,
    /// the label values, back to back
    text: String,
    /// where each label value ends in `text`
    ends: Vec<usize>,
}

impl LeLabels {
    /// Makes the labels those of `bounds`, rendering only the bounds that changed.
    fn update(&mut self, bounds: impl Iterator<Item = f64>) {
        if self.bounds.capacity() == 0 {
            // Most labels are short, such as `0.005` or `1e+06`.
            let count = bounds.size_hint().0;
            self.bounds.reserve(count);
            self.ends.reserve(count);
            self.text.reserve(count * 8);
        }
        let mut len = 0;
        for bound in bounds {
            // Bits, not `==`: 0.0 and -0.0 are equal but render differently.
            let kept = self
                .bounds
                .get(len)
                .is_some_and(|b| b.to_bits() == bound.to_bits());
            if !kept {
                self.truncate(len);
                self.bounds.push(bound);
                let Ok(()) = uwrite!(self.text, "{}", canonical_display(bound));
                self.ends.push(self.text.len());
            }
            len += 1;
        }
        self.truncate(len);
    }

    fn truncate(&mut self, len: usize) {
        self.bounds.truncate(len);
        self.ends.truncate(len);
        self.text.truncate(self.ends.last().copied().unwrap_or(0));
    }

    /// The `le` label value of the `i`-th bound.
    fn label(&self, i: usize) -> &str {
        let start = if i == 0 { 0 } else { self.ends[i - 1] };
        &self.text[start..self.ends[i]]
    }
}

fn write_histogram_samples<'p, T: Numeric + Copy + 'p, U: uWrite>(
    family: &mut FamilyContext<'_, U>,
    histogram: &Histogram<T>,
    points: impl Iterator<Item = (&'p HistogramDataPoint<T>, &'p str)>,
) -> Result<(), U::Error> {
    let ts = to_timestamp(histogram.time());
    let created = to_timestamp(histogram.start_time());
    assert_eq!(
        histogram.temporality(),
        Temporality::Cumulative,
        "Only cumulative Histograms are supported"
    );

    // om[impl metric.nointerleave] - each point's LabelSet is written contiguously
    for (point, attrs) in points {
        // c[impl histogram.created]
        uwriteln!(
            family.ctx.f,
            "{}_created{{{}}} {} {}"
            family.name,
            attrs,
            created,
            ts,
        )?;

        // c[impl histogram.bucket]
        family.ctx.le_labels.update(point.bounds());
        let le_labels = &family.ctx.le_labels;
        let bounds = le_labels.bounds.as_slice();
        // c[impl histogram.sum] - {name}_sum only if the sum is positive and monotonic
        // om[impl histogram.negative-threshold-nosum]
        // TODO: monitor if opentelmetry-sdk introduces positive f64 histograms?
        let has_sum = (T::is_unsigned() || point.min().as_ref().is_some_and(T::is_nonnegative))
            && bounds.iter().all(|bound| *bound >= 0.0);
        // om[impl metricpoint.nointerleave] - all value samples of one MetricPoint are written contiguously
        // om[impl histogram.count-iff-sum] - the +Inf bucket still carries the count
        if has_sum {
            // c[impl histogram.count] - except without a sum, where OpenMetrics forbids it
            uwriteln!(
                family.ctx.f,
                "{}_count{{{}}} {} {}",
                family.name,
                attrs,
                point.count().fast_display(),
                ts
            )?;
            uwriteln!(
                family.ctx.f,
                "{}_sum{{{}}} {} {}",
                family.name,
                attrs,
                point.sum().fast_display(),
                ts,
            )?;
        }

        if family.ctx.config.histogram_min_max {
            // Non-compliant but useful
            // TODO: Expose as a separate gauge?
            if let Some(min) = point.min() {
                uwriteln!(
                    family.ctx.f,
                    "{}_min{{{}}} {} {}",
                    family.name,
                    attrs,
                    min.fast_display(),
                    ts,
                )?;
            }
            if let Some(max) = point.max() {
                uwriteln!(
                    family.ctx.f,
                    "{}_max{{{}}} {} {}",
                    family.name,
                    attrs,
                    max.fast_display(),
                    ts,
                )?;
            }
        }

        let le_separator = if attrs.is_empty() { "" } else { "," };
        let mut cumulative_count = 0;
        for (i, count) in point.bucket_counts().take(bounds.len()).enumerate() {
            // c[impl histogram.bucket.cumulative]
            cumulative_count += count;
            // c[impl histogram.bucket.le]
            // om[impl histogram.sorted] - the SDK keeps bounds sorted, and `le`
            // values are canonical numbers
            uwrite!(
                // Not using write! here is a ~19% speedup
                family.ctx.f,
                "{}_bucket{{{}{}le=\"{}\"}} {} {}",
                family.name,
                attrs,
                le_separator,
                le_labels.label(i),
                cumulative_count.fast_display(),
                ts,
            )?;
            // om[impl exemplars.bucket-attachment] - attach the latest exemplar
            // whose value falls within this bucket; a bucket holds at most one.
            // c[impl exemplar.bucket-single]
            // c[impl histogram.bucket.exemplar] - a single exemplar per `le`
            // bucket, attached to no other `le`-labelled point
            write_bucket_exemplar(&mut family.ctx.f, bounds, i, point.exemplars())?;
            family.ctx.f.write_char('\n')?;
        }
        // om[impl histogram.inf-bucket]
        // c[impl histogram.bucket.inf]
        uwrite!(
            family.ctx.f,
            "{}_bucket{{{}{}le=\"+Inf\"}} {} {}",
            family.name,
            attrs,
            le_separator,
            point.count().fast_display(),
            ts,
        )?;
        // Exemplars above the last finite bound belong to the +Inf bucket.
        write_bucket_exemplar(&mut family.ctx.f, bounds, bounds.len(), point.exemplars())?;
        family.ctx.f.write_char('\n')?;
    }
    Ok(())
}

#[cfg(test)]
fn write_counter<T: Numeric + Copy, U: uWrite>(
    family: &mut FamilyContext<'_, U>,
    sum: &Sum<T>,
) -> Result<(), U::Error> {
    write_in_test_scope(family, sum)
}

impl<T: Numeric + Copy> DataPoints for Sum<T> {
    type Point = SumDataPoint<T>;
    const HISTOGRAM: bool = false;

    fn points(&self) -> impl Iterator<Item = &Self::Point> {
        self.data_points()
    }

    fn attributes(point: &Self::Point) -> impl Iterator<Item = &KeyValue> {
        point.attributes()
    }

    /// Keeps a counter point only if its total is a number of at least 0.
    /// The API does not validate increments, so a total can be negative.
    fn keeps_value(&self, metric: &str, point: &Self::Point) -> bool {
        let total = point.value().to_f64();
        if !self.is_monotonic() || total >= 0.0 {
            return true;
        }
        warn!(
            points = 1,
            "Dropping a point of {metric}: its total {total} is not a valid counter total"
        );
        false
    }

    fn write_samples<'p, U: uWrite>(
        &'p self,
        family: &mut FamilyContext<'_, U>,
        points: impl Iterator<Item = (&'p Self::Point, &'p str)>,
    ) -> Result<(), U::Error> {
        write_counter_samples(family, self, points)
    }
}

fn write_counter_samples<'p, T: Numeric + Copy + 'p, U: uWrite>(
    family: &mut FamilyContext<'_, U>,
    sum: &Sum<T>,
    points: impl Iterator<Item = (&'p SumDataPoint<T>, &'p str)>,
) -> Result<(), U::Error> {
    assert_eq!(
        sum.temporality(),
        opentelemetry_sdk::metrics::Temporality::Cumulative,
        "Only cumulative sums are supported"
    );

    let ts = to_timestamp(sum.time());
    let created = to_timestamp(sum.start_time());

    if sum.is_monotonic() {
        for (point, attrs) in points {
            // c[impl sum.created]
            // om[impl counter.suffix]
            uwriteln!(
                family.ctx.f,
                "{}_created{{{}}} {} {}",
                family.name,
                attrs,
                created,
                ts
            )?;
            // c[impl sum.total-suffix] - the family name is bare (see
            // `extract_type_unit_and_name`), so the `_total` suffix is always
            // appended to the value sample.
            uwrite!(
                family.ctx.f,
                "{}_total{{{}}} {} {}",
                family.name,
                attrs,
                point.value().fast_display(),
                ts,
            )?;
            // c[impl exemplar.types] - exemplars on monotonic sums are converted
            write_exemplar(&mut family.ctx.f, point.exemplars())?;
            family.ctx.f.write_char('\n')?;
        }
    } else {
        for (point, attrs) in points {
            // c[impl exemplar.types] - exemplars on non-monotonic sums (gauges)
            // are dropped
            uwrite!(
                family.ctx.f,
                "{}{{{}}} {} {}",
                family.name,
                attrs,
                point.value().fast_display(),
                ts,
            )?;
            family.ctx.f.write_char('\n')?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn write_gauge<T: Numeric + Copy, U: uWrite>(
    family: &mut FamilyContext<'_, U>,
    gauge: &Gauge<T>,
) -> Result<(), U::Error> {
    write_in_test_scope(family, gauge)
}

impl<T: Numeric + Copy> DataPoints for Gauge<T> {
    type Point = GaugeDataPoint<T>;
    const HISTOGRAM: bool = false;

    fn points(&self) -> impl Iterator<Item = &Self::Point> {
        self.data_points()
    }

    fn attributes(point: &Self::Point) -> impl Iterator<Item = &KeyValue> {
        point.attributes()
    }

    fn write_samples<'p, U: uWrite>(
        &'p self,
        family: &mut FamilyContext<'_, U>,
        points: impl Iterator<Item = (&'p Self::Point, &'p str)>,
    ) -> Result<(), U::Error> {
        let ts = to_timestamp(self.time());
        for (point, attrs) in points {
            // c[impl exemplar.types] - exemplars on gauges are dropped
            uwrite!(
                family.ctx.f,
                "{}{{{}}} {} {}",
                family.name,
                attrs,
                point.value().fast_display(),
                ts,
            )?;
            family.ctx.f.write_char('\n')?;
        }
        Ok(())
    }
}

/// Makes an `otel_scope_name` attribute with the specified `scope_name` if
/// the config enables `otel_scope_info`.
// c[impl scope.labels-on-points]
#[inline]
fn make_scope_name_attrs(
    config: Config,
    scope_name: &str,
    scope_version: Option<&str>,
) -> Vec<KeyValue> {
    // TODO: Get rid of the to_owned here, by not going through KeyValue
    if config.scope_info_enabled {
        Some(KeyValue::new("otel_scope_name", scope_name.to_owned()))
            .into_iter()
            .chain(scope_version.map(|v| KeyValue::new("otel_scope_version", v.to_owned())))
            .collect()
    } else {
        Vec::new()
    }
}

/// The labels the converter adds to a point itself, so no point attribute may
/// be written under one of these names.
fn reserved_labels(config: Config, histogram: bool) -> &'static [&'static str] {
    // om[impl histogram.nole] - `le` is reserved for the bucket threshold
    match (config.scope_info_enabled, histogram) {
        (true, true) => &["le", "otel_scope_name", "otel_scope_version"],
        (true, false) => &["otel_scope_name", "otel_scope_version"],
        (false, true) => &["le"],
        (false, false) => &[],
    }
}

/// Whether a point of `metric` keeps the `reserved` labels intact. A point with
/// an attribute that would be written as one of them is dropped with a warning.
// om[impl labels.unique] - an attribute must not repeat a label the converter adds
fn keeps_reserved_labels<'a>(
    metric: &str,
    mut attrs: impl Iterator<Item = &'a KeyValue>,
    reserved: &[&'static str],
) -> bool {
    if reserved.is_empty() {
        return true;
    }
    let shadowing = attrs.find_map(|kv| {
        reserved
            .iter()
            .find(|label| sanitizes_to(kv.key.as_str(), label))
            .map(|label| (kv, *label))
    });
    let Some((kv, label)) = shadowing else {
        return true;
    };
    // c[impl metadata.drop-warn]
    warn!(
        points = 1,
        "Dropping a point of {metric}: its attribute {:?} would be written as the reserved label `{label}`",
        kv.key.as_str()
    );
    false
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
