use std::borrow::Cow;
use std::fmt::Write;
use std::hash::{DefaultHasher, Hasher};
use std::time::SystemTime;

use crate::format::Numeric;
use opentelemetry::{Array, InstrumentationScope, Key, KeyValue, SpanId, TraceId, Value};
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, Exemplar, Gauge, GaugeDataPoint, Histogram, HistogramDataPoint, MetricData,
    ResourceMetrics, Sum, SumDataPoint,
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
struct Context<'f, W: uWrite> {
    /// the output [Write] reference
    f: W,
    /// the conversion configuration
    config: Config,
    /// the label sets of the current family's points, back to back
    attr_buffer: String,
    /// the current family's points, each with its label set in `attr_buffer`
    labeled_points: Vec<LabeledPoint>,
    /// scratch space for sanitized label names, reused across points
    label_names: String,
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
            labeled_points: Vec::new(),
            label_names: String::with_capacity(64),
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

        let Plan { names, mut members } = Plan::new(&mut ctx, &scopes);
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
                    "Dropping family {name}: one of its sample names is already written by another family"
                );
                continue;
            }

            // c[impl metadata.conflict-type] - the plan orders by type within a
            // family, so the first member's type wins
            let typ = family[0].typ;
            ctx.name.clear();
            ctx.name.push_str(name);
            ctx.typ = typ;
            // c[impl metadata.conflict-unit-help] - the first UNIT and HELP are kept
            ctx.unit = family
                .iter_mut()
                .filter(|member| member.typ == typ)
                .find_map(|member| member.unit.take());
            let description = family
                .iter()
                .filter(|member| member.typ == typ)
                .map(|member| member.metric.description())
                .find(|description| !description.is_empty())
                .unwrap_or_default();
            // om[impl metadata.unique] - one header per family, however many metrics it holds
            write_header(&mut ctx, description)?;

            // c[impl metadata.conflict-keep-points] - every member of the family's type is written
            let mut written = Vec::with_capacity(family.len());
            for member in family.iter() {
                if member.typ != typ {
                    // c[impl metadata.drop-warn]
                    warn!(
                        "Dropping metric {:?}: family {name} is already of type {typ}, not {}",
                        member.metric.name(),
                        member.typ
                    );
                    continue;
                }
                let scope = (member.scope.name(), member.scope.version());
                written.push((scope, member.metric.data()));
            }
            write_points(&mut ctx, &written)?;
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
    write_attrs_tuple(f, &mut String::with_capacity(64), resource.iter())?;
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
    fn new(
        ctx: &mut Context<'_, impl uWrite<Error = std::fmt::Error>>,
        scopes: &[&'m ScopeMetrics],
    ) -> Self {
        let mut names = String::new();
        let mut members = Vec::new();
        for scope_metrics in scopes {
            let scope = scope_metrics.scope();
            let scope_hash = hash_attrs(scope.attributes());
            for metric in scope_metrics.metrics() {
                if extract_type_unit_and_name(ctx, metric) {
                    let start = names.len();
                    names.push_str(&ctx.name);
                    members.push(Member {
                        family: (start, names.len()),
                        typ: ctx.typ,
                        unit: ctx.unit.take(),
                        scope,
                        scope_hash,
                        metric,
                    });
                } else {
                    // c[impl metadata.drop-warn]
                    warn!("Unsupported metric type {metric:?}");
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
                .then_with(|| number_kind(a.metric.data()).cmp(&number_kind(b.metric.data())))
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
                runs.iter().for_each(|&run| taken[run] = true);
            }
            clashes
        })
        .collect()
}

/// Orders metrics of one name that differ only in their number type.
fn number_kind(data: &AggregatedMetrics) -> u8 {
    match data {
        AggregatedMetrics::F64(_) => 0,
        AggregatedMetrics::U64(_) => 1,
        AggregatedMetrics::I64(_) => 2,
    }
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

/// Write the metadata of the family named in `ctx.name`, with `ctx.typ` and `ctx.unit`.
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
    config: Config,
) -> Result<(), U::Error> {
    // c[impl scope.info]
    f.write_str("# TYPE otel_scope info\n")?;

    let mut label_names = String::with_capacity(64);
    for scope in metrics {
        // c[impl scope.name-version]
        let otel_attrs =
            make_scope_name_attrs(config, scope.scope().name(), scope.scope().version());
        f.write_str("otel_scope_info{")?;
        // c[impl scope.attribute-labels]
        write_attrs_with(
            f,
            &mut label_names,
            otel_attrs.iter().chain(scope.scope().attributes()),
        )?;
        // om[impl info.value]
        f.write_str("} 1\n")?;
    }
    Ok(())
}

/// Write all data points for this metric, in the scope set in `ctx`.
fn write_values<U: uWrite>(
    ctx: &mut Context<'_, U>,
    metric: &AggregatedMetrics,
) -> Result<(), U::Error> {
    let scope = (ctx.scope_name, ctx.scope_version);
    write_points(ctx, &[(scope, metric)])
}

/// The name and version of an instrumentation scope.
type ScopeLabels<'s> = (&'s str, Option<&'s str>);

/// A point of a family, with its label set rendered into `Context::attr_buffer`.
#[derive(Clone, Copy)]
struct LabeledPoint {
    /// index of the metric in the family's members
    member: usize,
    /// index of the point in its metric's data points
    point: usize,
    /// byte range of the label set
    start: usize,
    end: usize,
    /// the label set's [`sort_key`], set while sorting
    key: u64,
}

/// Writes the points of one family. `members` holds each metric of the family
/// with the scope it was recorded in.
///
/// Each metric's points are written in the order of their label sets. A point
/// whose label set another point of the family already has is dropped.
// c[impl resource.attrs] - resource attributes are dropped: labels on metric
// families carry only point and scope attributes, never resource attributes
fn write_points<M: FamilyMember, U: uWrite>(
    ctx: &mut Context<'_, U>,
    members: &[(ScopeLabels<'_>, &M)],
) -> Result<(), U::Error> {
    // Taken out of `ctx` while the members write, so their capacity is reused.
    let mut text = std::mem::take(&mut ctx.attr_buffer);
    let mut points = std::mem::take(&mut ctx.labeled_points);
    text.clear();
    points.clear();

    for (member, (scope, data)) in members.iter().enumerate() {
        data.render_labels(ctx, *scope, member, &mut text, &mut points);
    }
    drop_repeated_label_sets(&ctx.name, &text, &mut points, |member| {
        members[member].1.hash_points()
    });

    let mut written = Ok(());
    let mut rest = points.as_slice();
    for (member, (_, data)) in members.iter().enumerate() {
        let len = rest.iter().take_while(|p| p.member == member).count();
        let (own, later) = rest.split_at(len);
        rest = later;
        written = data.write_labeled(ctx, own, &text);
        if written.is_err() {
            break;
        }
    }

    ctx.attr_buffer = text;
    ctx.labeled_points = points;
    written
}

/// Sorts `points` by label set and keeps one point per label set, then groups
/// the kept points by member.
///
/// Of the points sharing a label set, the one of the first member is kept.
/// Within one member, the SDK's point order is arbitrary, so the point with the
/// smallest [`hash_attrs`] is kept. `point_hashes` gives those hashes for all
/// points of a member, and is called only for such repeats.
// om[impl metricfamily.unique-labelset]
fn drop_repeated_label_sets(
    family: &str,
    text: &str,
    points: &mut Vec<LabeledPoint>,
    point_hashes: impl Fn(usize) -> Vec<u64>,
) {
    sort_by_label_set(text.as_bytes(), points, 0, 0);
    let label_set = |p: &LabeledPoint| &text[p.start..p.end];

    let mut hashes: Vec<(usize, Vec<u64>)> = Vec::new();
    let mut kept = 0;
    let mut start = 0;
    while start < points.len() {
        let labels = label_set(&points[start]);
        let len = points[start..]
            .iter()
            .take_while(|p| label_set(p) == labels)
            .count();
        let run = &points[start..start + len];
        start += len;

        let first_member = run[0].member;
        let candidates = run.iter().take_while(|p| p.member == first_member);
        let survivor = if candidates.clone().nth(1).is_none() {
            run[0]
        } else {
            let index = match hashes.iter().position(|(m, _)| *m == first_member) {
                Some(index) => index,
                None => {
                    hashes.push((first_member, point_hashes(first_member)));
                    hashes.len() - 1
                }
            };
            let member_hashes = &hashes[index].1;
            *candidates
                .min_by_key(|p| (member_hashes[p.point], p.point))
                .expect("a run is never empty")
        };
        let labels = &text[survivor.start..survivor.end];
        for _ in 1..len {
            // c[impl metadata.drop-warn]
            warn!("Dropping a point of {family}: its label set {{{labels}}} is already written");
        }
        points[kept] = survivor;
        kept += 1;
    }
    points.truncate(kept);

    if !points.is_sorted_by_key(|p| p.member) {
        points.sort_by_key(|p| p.member);
    }
}

/// Sorts `points` by label set, and the points of one label set by member.
/// Points of one member with the same label set stay in any order. All label
/// sets in `points` agree on their first `skip` bytes.
///
/// Points are sorted by 8 bytes of their label set at a time. Before each
/// round, the bytes that all label sets of the group share are skipped, so a
/// long common prefix is read once instead of in every comparison.
fn sort_by_label_set(text: &[u8], points: &mut [LabeledPoint], skip: usize, depth: usize) {
    /// Rounds after which the rest is sorted by comparing whole label sets.
    const MAX_DEPTH: usize = 16;

    let Some((first, others)) = points.split_first() else {
        return;
    };
    let after = |p: &LabeledPoint, skip: usize| &text[p.start + skip..p.end];
    let common = others.iter().fold(after(first, skip).len(), |common, p| {
        common_prefix_len(&after(first, skip)[..common], after(p, skip))
    });
    let skip = skip + common;

    if depth == MAX_DEPTH {
        points.sort_unstable_by(|a, b| {
            after(a, skip)
                .cmp(after(b, skip))
                .then(a.member.cmp(&b.member))
        });
        return;
    }

    for point in points.iter_mut() {
        point.key = sort_key(after(point, skip));
    }
    // The key pads short label sets with zeros, and label values may hold NUL
    // bytes. Of two label sets with the same key, the one with fewer bytes
    // left is the smaller, unless both go on past the key.
    let left = |p: &LabeledPoint| (p.end - p.start - skip).min(8);
    let rank =
        |p: &LabeledPoint| (u128::from(p.key) << 64) | ((left(p) as u128) << 60) | p.member as u128;
    points.sort_unstable_by_key(rank);

    let mut start = 0;
    while start < points.len() {
        let group = |p: &LabeledPoint| (p.key, left(p));
        let key = group(&points[start]);
        let len = points[start..]
            .iter()
            .take_while(|p| group(p) == key)
            .count();
        // Label sets that go on past an equal key are ordered by what follows.
        if len > 1 && key.1 == 8 {
            sort_by_label_set(text, &mut points[start..start + len], skip + 8, depth + 1);
        }
        start += len;
    }
}

/// The number of leading bytes `a` and `b` share, compared 8 bytes at a time.
fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    let len = a.len().min(b.len());
    let mut i = 0;
    while i + 8 <= len {
        let word = |bytes: &[u8]| u64::from_le_bytes(bytes[i..i + 8].try_into().expect("8 bytes"));
        let difference = word(a) ^ word(b);
        if difference != 0 {
            return i + difference.trailing_zeros() as usize / 8;
        }
        i += 8;
    }
    i + std::iter::zip(&a[i..len], &b[i..len])
        .take_while(|(x, y)| x == y)
        .count()
}

/// The first 8 bytes of `bytes` as a number, padded with zeros. When the keys
/// of two byte strings differ, they order the strings as their bytes do.
fn sort_key(bytes: &[u8]) -> u64 {
    let mut key = [0; 8];
    let len = bytes.len().min(8);
    key[..len].copy_from_slice(&bytes[..len]);
    u64::from_be_bytes(key)
}

/// Metric data that contributes points to a family.
trait FamilyMember {
    /// Renders the label set of each point onto `text` and records the point
    /// in `points`. A point that would shadow a reserved label is skipped.
    fn render_labels<U: uWrite>(
        &self,
        ctx: &mut Context<'_, U>,
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
        ctx: &mut Context<'_, U>,
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

    /// Writes the samples of `points`, each with its rendered label set.
    fn write_samples<'p, U: uWrite>(
        &'p self,
        ctx: &mut Context<'_, U>,
        points: impl Iterator<Item = (&'p Self::Point, &'p str)>,
    ) -> Result<(), U::Error>;
}

impl<D: DataPoints> FamilyMember for D {
    fn render_labels<U: uWrite>(
        &self,
        ctx: &mut Context<'_, U>,
        (scope_name, scope_version): ScopeLabels<'_>,
        member: usize,
        text: &mut String,
        points: &mut Vec<LabeledPoint>,
    ) {
        // c[impl scope.labels-on-points]
        let scope_name_attrs = make_scope_name_attrs(ctx.config, scope_name, scope_version);
        let reserved = reserved_labels(ctx.config, D::HISTOGRAM);
        let data_points = self.points();
        let count = data_points.size_hint().0;
        points.reserve(count);
        for (index, point) in data_points.enumerate() {
            if !keeps_reserved_labels(&ctx.name, D::attributes(point), reserved) {
                continue;
            }
            let start = text.len();
            let Ok(()) = write_attrs_with(
                text,
                &mut ctx.label_names,
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
        ctx: &mut Context<'_, U>,
        points: &[LabeledPoint],
        text: &str,
    ) -> Result<(), U::Error> {
        let data_points: Vec<_> = self.points().collect();
        self.write_samples(
            ctx,
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
        ctx: &mut Context<'_, U>,
        scope: ScopeLabels<'_>,
        member: usize,
        text: &mut String,
        points: &mut Vec<LabeledPoint>,
    ) {
        with_data_points!(self, data => data.render_labels(ctx, scope, member, text, points))
    }

    fn hash_points(&self) -> Vec<u64> {
        with_data_points!(self, data => data.hash_points())
    }

    fn write_labeled<U: uWrite>(
        &self,
        ctx: &mut Context<'_, U>,
        points: &[LabeledPoint],
        text: &str,
    ) -> Result<(), U::Error> {
        with_data_points!(self, data => data.write_labeled(ctx, points, text))
    }
}

/// Whether `value` belongs to the `i`-th bucket of `bounds`, which spans the
/// half-open range `(bounds[i - 1], bounds[i]]`. The index `bounds.len()`
/// denotes the `+Inf` bucket, holding everything above the last finite bound.
fn falls_in_bucket<T: Numeric>(bounds: &[f64], i: usize, value: &T) -> bool {
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
fn write_bucket_exemplar<'a, E: ExemplarPoint + 'a, U: uWrite>(
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

fn write_histogram<T: Numeric + Copy, U: uWrite>(
    ctx: &mut Context<'_, U>,
    histogram: &Histogram<T>,
) -> Result<(), U::Error> {
    let scope = (ctx.scope_name, ctx.scope_version);
    write_points(ctx, &[(scope, histogram)])
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
        ctx: &mut Context<'_, U>,
        points: impl Iterator<Item = (&'p Self::Point, &'p str)>,
    ) -> Result<(), U::Error> {
        write_histogram_samples(ctx, self, points)
    }
}

fn write_histogram_samples<'p, T: Numeric + Copy, U: uWrite>(
    ctx: &mut Context<'_, U>,
    histogram: &Histogram<T>,
    points: impl Iterator<Item = (&'p HistogramDataPoint<T>, &'p str)>,
) -> Result<(), U::Error>
where
    T: 'p,
{
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
            ctx.f,
            "{}_created{{{}}} {} {}"
            ctx.name,
            attrs,
            created,
            ts,
        )?;

        // c[impl histogram.bucket]
        let bounds: Vec<_> = point.bounds().collect();
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
                ctx.f,
                "{}_count{{{}}} {} {}",
                ctx.name,
                attrs,
                point.count().fast_display(),
                ts
            )?;
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

        let le_separator = if attrs.is_empty() { "" } else { "," };
        let mut cumulative_count = 0;
        for (i, (bound, count)) in std::iter::zip(&bounds, point.bucket_counts()).enumerate() {
            // c[impl histogram.bucket.cumulative]
            cumulative_count += count;
            // c[impl histogram.bucket.le]
            uwrite!(
                // Not using write! here is a ~19% speedup
                ctx.f,
                "{}_bucket{{{}{}le=\"{}\"}} {} {}",
                ctx.name,
                attrs,
                le_separator,
                bound.fast_display(),
                cumulative_count.fast_display(),
                ts,
            )?;
            // om[impl exemplars.bucket-attachment] - attach the latest exemplar
            // whose value falls within this bucket; a bucket holds at most one.
            // c[impl exemplar.bucket-single]
            // c[impl histogram.bucket.exemplar] - a single exemplar per `le`
            // bucket, attached to no other `le`-labelled point
            write_bucket_exemplar(&mut ctx.f, &bounds, i, point.exemplars())?;
            ctx.f.write_char('\n')?;
        }
        // om[impl histogram.inf-bucket]
        // c[impl histogram.bucket.inf]
        uwrite!(
            ctx.f,
            "{}_bucket{{{}{}le=\"+Inf\"}} {} {}",
            ctx.name,
            attrs,
            le_separator,
            point.count().fast_display(),
            ts,
        )?;
        // Exemplars above the last finite bound belong to the +Inf bucket.
        write_bucket_exemplar(&mut ctx.f, &bounds, bounds.len(), point.exemplars())?;
        ctx.f.write_char('\n')?;
    }
    Ok(())
}

fn write_counter<T: Numeric + Copy, U: uWrite>(
    ctx: &mut Context<'_, U>,
    sum: &Sum<T>,
) -> Result<(), U::Error> {
    let scope = (ctx.scope_name, ctx.scope_version);
    write_points(ctx, &[(scope, sum)])
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

    fn write_samples<'p, U: uWrite>(
        &'p self,
        ctx: &mut Context<'_, U>,
        points: impl Iterator<Item = (&'p Self::Point, &'p str)>,
    ) -> Result<(), U::Error> {
        write_counter_samples(ctx, self, points)
    }
}

fn write_counter_samples<'p, T: Numeric + Copy, U: uWrite>(
    ctx: &mut Context<'_, U>,
    sum: &Sum<T>,
    points: impl Iterator<Item = (&'p SumDataPoint<T>, &'p str)>,
) -> Result<(), U::Error>
where
    T: 'p,
{
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
        for (point, attrs) in points {
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
    let scope = (ctx.scope_name, ctx.scope_version);
    write_points(ctx, &[(scope, gauge)])
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
        ctx: &mut Context<'_, U>,
        points: impl Iterator<Item = (&'p Self::Point, &'p str)>,
    ) -> Result<(), U::Error> {
        let ts = to_timestamp(self.time());
        for (point, attrs) in points {
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
        "Dropping a point of {metric}: its attribute {:?} would be written as the reserved label `{label}`",
        kv.key.as_str()
    );
    false
}

/// Whether `key` is written as the label name `label`, compared without allocating.
fn sanitizes_to(key: &str, label: &str) -> bool {
    /// Accepts exactly the text of the label, piece by piece.
    struct Expect<'l>(&'l str);

    impl uWrite for Expect<'_> {
        type Error = ();

        fn write_str(&mut self, s: &str) -> Result<(), ()> {
            self.0 = self.0.strip_prefix(s).ok_or(())?;
            Ok(())
        }
    }

    // Sanitizing keeps a leading letter, and every reserved label starts with one.
    if key.as_bytes().first() != label.as_bytes().first() {
        return false;
    }
    let mut expect = Expect(label);
    write_sanitized_name(&mut expect, key, NameKind::AttributeLabel).is_ok() && expect.0.is_empty()
}

/// Write the attribute string for attrs. Does not write curly braces.
fn write_attrs<'a, I: Iterator<Item = &'a KeyValue>, U: uWrite>(
    f: &mut U,
    attrs: I,
) -> Result<(), U::Error> {
    write_attrs_with(f, &mut String::new(), attrs)
}

/// Like [`write_attrs`], but reuses `label_names` as scratch space, so the
/// per-point writers allocate nothing new.
fn write_attrs_with<'a, I: Iterator<Item = &'a KeyValue>, U: uWrite>(
    f: &mut U,
    label_names: &mut String,
    attrs: I,
) -> Result<(), U::Error> {
    write_attrs_tuple(f, label_names, attrs.map(|kv| (&kv.key, &kv.value)))
}

fn write_attrs_tuple<'a, I: Iterator<Item = (&'a Key, &'a Value)>, U: uWrite>(
    f: &mut U,
    label_names: &mut String,
    attrs: I,
) -> Result<(), U::Error> {
    // c[impl mattrs.to-labels] - attribute keys/values are emitted as label name/value pairs
    // Each key is sanitized once into `label_names`; a label holds its byte range there.
    label_names.clear();
    let mut labels: Vec<(usize, usize, &Key, &Value)> = attrs
        .map(|(key, value)| {
            let start = label_names.len();
            let Ok(()) = write_sanitized_name(label_names, key.as_str(), NameKind::AttributeLabel);
            (start, label_names.len(), key, value)
        })
        .collect();
    // c[impl mattrs.key-sanitize.collisions] - colliding labels become adjacent,
    // in the order of their original keys
    labels.sort_unstable_by(|a, b| {
        label_names[a.0..a.1]
            .cmp(&label_names[b.0..b.1])
            .then_with(|| a.2.cmp(b.2))
    });

    let mut previous_name: Option<&str> = None;
    for &(start, end, _, value) in &labels {
        let name = &label_names[start..end];
        if previous_name == Some(name) {
            // c[impl mattrs.key-sanitize.collisions] - values of one label are joined with `;`
            f.write_char(';')?;
        } else {
            if previous_name.is_some() {
                f.write_str("\",")?;
            }
            f.write_str(name)?;
            f.write_str("=\"")?;
        }
        // c[impl mattrs.type-conversion] - non-string values are stringified
        // c[impl attrs.stringify] - go through opentelemetry-sdk's `as_str`, TODO: implement our own to avoid allocating
        write_escaped(f, &value.as_str())?;
        previous_name = Some(name);
    }
    if previous_name.is_some() {
        f.write_char('"')?;
    }
    Ok(())
}

/// Calculates a hash of the [`KeyValue`] pairs which is invariant under reordering of the [`KeyValue`]s within the [`Iterator`].
fn hash_attrs<'a, I: Iterator<Item = &'a KeyValue>>(attrs: I) -> u64 {
    let mut hash = 0;
    for kv in attrs {
        let mut hasher = DefaultHasher::default();
        hasher.write(kv.key.as_str().as_bytes());
        // The type tells apart values that stringify alike, such as 1 and "1".
        hasher.write_u8(value_type(&kv.value));
        // TODO: replace `as_str` to avoid allocations
        hasher.write(kv.value.as_str().as_bytes());
        hash ^= hasher.finish(); // XOR to be order-invariant
    }
    hash
}

/// A number for each type of attribute value.
fn value_type(value: &Value) -> u8 {
    match value {
        Value::Bool(_) => 0,
        Value::I64(_) => 1,
        Value::F64(_) => 2,
        Value::String(_) => 3,
        Value::Array(Array::Bool(_)) => 4,
        Value::Array(Array::I64(_)) => 5,
        Value::Array(Array::F64(_)) => 6,
        Value::Array(Array::String(_)) => 7,
        _ => 8,
    }
}

/// Writes to `f` the contents of `value` as an escaped string. Does not put quotes around the value.
/// The chars to escape are `\`, `"` and `\n`.
// om[impl escaping.chars]
fn write_escaped<U: uWrite>(f: &mut U, value: &str) -> Result<(), U::Error> {
    #[inline]
    fn next_escape_char(bytes: &[u8]) -> Option<usize> {
        memchr::memchr3(b'\\', b'"', b'\n', bytes)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    // Allowed characters are written in runs, one `write_str` per run. They are
    // all ASCII, so scanning bytes is enough: the bytes of a multi-byte char are
    // all invalid and collapse into one `_`, and every run boundary sits next to
    // an ASCII byte, which is always a char boundary.
    let mut run_start = 0;
    for (i, &byte) in name.as_bytes().iter().enumerate() {
        // Allowed characters are `a-z A-Z 0-9 : _`, except for ':' in labels
        // Invalid characters in the metric name MUST be replaced with the `_` character.
        if byte.is_ascii_alphanumeric() || (byte == b':' && kind == NameKind::Metric) {
            if previous_was_underscore {
                run_start = i;
                previous_was_underscore = false;
            }
        } else if !previous_was_underscore {
            f.write_str(&name[run_start..i])?;
            f.write_char('_')?;
            previous_was_underscore = true;
        }
    }
    if !previous_was_underscore {
        f.write_str(&name[run_start..])?;
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

fn write_exemplar<'a, E: ExemplarPoint + 'a, U: uWrite>(
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
