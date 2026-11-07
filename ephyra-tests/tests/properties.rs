//! Properties of ottotom's conversion, checked on metrics produced by a real
//! SDK pipeline from generated scenarios. Each assertion carries the tracey
//! annotation of the rule it verifies: `om[...]` for docs/spec/openmetrics.md,
//! `c[...]` for docs/spec/conversion.md.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ResourceMetrics};
use ottotom::convert::WriteOpenMetrics;
use ottotom_ephyra_tests::exposition::{self, FamilySample, Line, Number, Sample};
use ottotom_ephyra_tests::scenario::{Scenario, scenario};
use ottotom_ephyra_tests::sdk::{self, Order};

fn convert(scenario: &Scenario, metrics: &ResourceMetrics) -> String {
    metrics
        .to_openmetrics_string_with_config(sdk::config(scenario))
        .expect("writing into a String cannot fail")
}

fn is_label_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_metric_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == ':')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

/// Whether the samples of some family in `text` carry different sets of label
/// names. In a histogram, `le` is left out, because only buckets carry it.
fn label_names_differ_within_a_family(text: &str) -> bool {
    let Ok(lines) = exposition::parse(text) else {
        return false;
    };
    let mut names_by_family: HashMap<&str, Vec<&str>> = HashMap::new();
    exposition::samples_by_family(&lines).iter().any(|s| {
        let mut names: Vec<&str> = s
            .sample
            .labels
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        names.retain(|name| s.typ != "histogram" || *name != "le");
        names.sort_unstable();
        names_by_family
            .entry(s.family)
            .or_insert_with(|| names.clone())
            != &names
    })
}

#[ephyra::test(gen = scenario(), tests = 2000)]
fn output_is_accepted_by_openmetrics_parser(scenario: Scenario) {
    let text = convert(&scenario, &sdk::collect(&scenario, Order::AsWritten));
    // The OpenMetrics ABNF allows a raw CR in strings, but this parser rejects
    // one. Labels and metric names are sanitized, so a CR can only sit in a
    // label value or HELP text, and replacing it keeps the structure.
    let parseable = text.replace('\r', "r");
    match openmetrics_parser::openmetrics::parse_openmetrics(&parseable) {
        Ok(_) => {}
        // "A MetricSet ... MAY be empty", but this parser rejects a lone `# EOF`.
        Err(_) if parseable == "# EOF\n" => {}
        // Rejections that no OpenMetrics sentence supports:
        // - "Metrics with the same name for a given MetricFamily SHOULD have
        //   the same set of label names" is a SHOULD, enforced as a MUST.
        // - Histograms may have a unit (`foo_seconds` is the usual example).
        // - A counter named `x_created` has samples `x_created_total` and
        //   `x_created_created`; the parser assigns them to a family `x`.
        Err(openmetrics_parser::ParseError::InvalidMetric(message))
            if message.contains("different label sets")
                || message.ends_with("metrics can't have units")
                || message.starts_with("Invalid Name in metric family") => {}
        // - The same SHOULD again: once label names differ, the parser compares
        //   label values only, so `{a="1"}` and `{b="1"}` count as the same
        //   Metric, repeated or interleaved. `output_is_well_formed` checks
        //   both rules on the full label sets.
        Err(openmetrics_parser::ParseError::DuplicateMetric)
            if label_names_differ_within_a_family(&parseable) => {}
        Err(openmetrics_parser::ParseError::InvalidMetric(message))
            if message.starts_with("Interwoven labelsets")
                && label_names_differ_within_a_family(&parseable) => {}
        Err(error) => {
            let kind = match &error {
                openmetrics_parser::ParseError::ParseError(message) => {
                    let expected = message.lines().last().unwrap_or_default().trim();
                    format!("parse error, {expected}")
                }
                openmetrics_parser::ParseError::DuplicateMetric => "duplicate metric".into(),
                openmetrics_parser::ParseError::InvalidMetric(message) => {
                    format!(
                        "invalid metric, {}",
                        message.split(':').next().unwrap_or_default()
                    )
                }
            };
            ephyra::signature(format!("openmetrics-parser: {kind}"));
            panic!("openmetrics-parser rejected the output: {error:?}\n{text}");
        }
    }
}

#[ephyra::test(gen = scenario(), tests = 2000)]
fn output_is_well_formed(scenario: Scenario) {
    let text = convert(&scenario, &sdk::collect(&scenario, Order::AsWritten));
    let lines =
        exposition::parse(&text).unwrap_or_else(|e| panic!("unreadable output: {e}\n{text}"));
    let samples = exposition::samples_by_family(&lines);

    // om[verify metadata.unique]
    // c[verify metadata.no-duplicates]
    let mut seen = HashSet::new();
    for line in &lines {
        let key = match line {
            Line::Type { family, .. } => ("TYPE", family),
            Line::Unit { family, .. } => ("UNIT", family),
            Line::Help { family, .. } => ("HELP", family),
            _ => continue,
        };
        assert!(
            seen.insert(key),
            "two {} lines for family {:?}\n{text}",
            key.0,
            key.1
        );
    }

    // om[verify metricfamily.name-clash]
    let mut owner: HashMap<String, &str> = HashMap::new();
    for line in &lines {
        if let Line::Type { family, typ } = line {
            for name in exposition::sample_names(family, typ) {
                if let Some(other) = owner.insert(name.clone(), family) {
                    assert_eq!(
                        other, family,
                        "families {other:?} and {family:?} both own sample name {name:?}\n{text}"
                    );
                }
            }
        }
    }

    for FamilySample {
        family,
        typ,
        sample,
    } in &samples
    {
        assert!(
            is_metric_name(&sample.name),
            "invalid metric name {:?}\n{text}",
            sample.name
        );
        for (name, _) in &sample.labels {
            // om[related labels.reserved] - names that start with `_` are a documented deviation.
            assert!(
                is_label_name(name),
                "invalid label name {name:?} on {:?}\n{text}",
                sample.name
            );
        }
        // om[verify labels.unique]
        let names: HashSet<_> = sample.labels.iter().map(|(n, _)| n).collect();
        assert_eq!(
            names.len(),
            sample.labels.len(),
            "repeated label name on {:?}: {:?}\n{text}",
            sample.name,
            sample.labels
        );
        // om[verify histogram.nole]
        if *typ == "histogram" && sample.name != format!("{family}_bucket") {
            assert!(
                sample.label("le").is_none(),
                "`le` label on {:?}\n{text}",
                sample.name
            );
        }
        assert!(
            exposition::sample_names(family, typ).contains(&sample.name),
            "sample {:?} does not belong to its {typ} family {family:?}\n{text}",
            sample.name
        );
    }

    // om[verify metric.nointerleave] - the samples of one label set are adjacent
    // within their family. In a histogram, `le` tells the buckets of one Metric
    // apart, so it is left out there.
    let mut finished = HashSet::new();
    let mut current: Option<(&str, Vec<(&str, &str)>)> = None;
    for FamilySample {
        family,
        typ,
        sample,
    } in &samples
    {
        let mut labels: Vec<(&str, &str)> = sample
            .labels
            .iter()
            .filter(|(name, _)| *typ != "histogram" || name != "le")
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        labels.sort_unstable();
        let metric = (*family, labels);
        if current.as_ref() != Some(&metric) {
            if let Some(previous) = current.replace(metric.clone()) {
                finished.insert(previous);
            }
            assert!(
                !finished.contains(&metric),
                "label set {:?} of family {:?} is interleaved\n{text}",
                metric.1,
                metric.0
            );
        }
    }

    // om[verify metricfamily.unique-labelset]
    let mut label_sets = HashSet::new();
    for FamilySample { sample, .. } in &samples {
        let mut labels = sample.labels.clone();
        labels.sort();
        assert!(
            label_sets.insert((sample.name.clone(), labels)),
            "repeated label set on {:?}: {:?}\n{text}",
            sample.name,
            sample.labels
        );
    }
}

/// The label name an attribute key is written as, derived from the OTel
/// conversion rules independently of ottotom: characters outside
/// `[a-zA-Z0-9_]` become `_`, runs of `_` collapse, and a leading digit gets a
/// `_` prefix.
fn label_name(key: &str) -> String {
    let mut name = String::new();
    for c in key.chars() {
        let c = if c.is_ascii_alphanumeric() { c } else { '_' };
        if !(c == '_' && name.ends_with('_')) {
            name.push(c);
        }
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name
}

/// What one SDK data point must turn into, keyed without any sanitized names:
/// the sample's role, the label values, and the value.
type Expected = (&'static str, Vec<String>, Number);

fn expected_points(scenario: &Scenario, metrics: &ResourceMetrics) -> (Vec<Expected>, Vec<String>) {
    fn int(v: impl Into<i128>) -> Number {
        Number::Int(v.into())
    }
    let mut points = Vec::new();
    let mut helps = Vec::new();
    for scope in metrics.scope_metrics() {
        let mut scope_labels = Vec::new();
        if scenario.scope_info {
            scope_labels.push(scope.scope().name().to_owned());
            scope_labels.extend(scope.scope().version().map(str::to_owned));
        }
        // c[verify mattrs.key-sanitize.collisions] - the values of keys that are
        // written as one label name are joined with `;`, in the order of the keys.
        let labels = |attrs: &mut dyn Iterator<Item = &opentelemetry::KeyValue>| {
            let mut by_label: BTreeMap<String, Vec<(&str, String)>> = BTreeMap::new();
            for kv in attrs {
                let value = kv.value.as_str().into_owned();
                by_label
                    .entry(label_name(kv.key.as_str()))
                    .or_default()
                    .push((kv.key.as_str(), value));
            }
            let mut values: Vec<String> = by_label
                .into_values()
                .map(|mut keyed| {
                    keyed.sort();
                    keyed
                        .into_iter()
                        .map(|(_, value)| value)
                        .collect::<Vec<_>>()
                        .join(";")
                })
                .collect();
            values.extend(scope_labels.iter().cloned());
            values.sort();
            values
        };
        for metric in scope.metrics() {
            if !metric.description().is_empty() {
                helps.push(metric.description().to_owned());
            }
            macro_rules! push_points {
                ($data:expr, $num:expr) => {
                    match $data {
                        MetricData::Gauge(g) => {
                            for p in g.data_points() {
                                points.push((
                                    "gauge",
                                    labels(&mut p.attributes()),
                                    $num(p.value()),
                                ));
                            }
                        }
                        MetricData::Sum(s) => {
                            let role = if s.is_monotonic() { "counter" } else { "gauge" };
                            for p in s.data_points() {
                                points.push((role, labels(&mut p.attributes()), $num(p.value())));
                            }
                        }
                        MetricData::Histogram(h) => {
                            for p in h.data_points() {
                                points.push((
                                    "histogram",
                                    labels(&mut p.attributes()),
                                    int(p.count() as u64),
                                ));
                            }
                        }
                        MetricData::ExponentialHistogram(_) => unreachable!("not configured"),
                    }
                };
            }
            match metric.data() {
                AggregatedMetrics::F64(data) => push_points!(data, Number::from_f64),
                AggregatedMetrics::U64(data) => push_points!(data, int),
                AggregatedMetrics::I64(data) => push_points!(data, int),
            }
        }
    }
    points.sort();
    helps.sort();
    (points, helps)
}

/// The role of `sample` if it carries a point's value. A histogram's count is
/// read from its `+Inf` bucket, which is written even when `_count` is not.
fn value_role(family: &str, typ: &str, sample: &Sample) -> Option<&'static str> {
    match typ {
        "gauge" if sample.name == family => Some("gauge"),
        "counter" if sample.name == format!("{family}_total") => Some("counter"),
        "histogram"
            if sample.name == format!("{family}_bucket") && sample.label("le") == Some("+Inf") =>
        {
            Some("histogram")
        }
        _ => None,
    }
}

/// Sums the `points` fields of the warnings emitted while it is the default
/// subscriber. The converter reports every point it drops this way.
struct DroppedPoints(Arc<AtomicUsize>);

impl tracing::Subscriber for DroppedPoints {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.is_event() && *metadata.level() == tracing::Level::WARN
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        struct Points(u64);

        impl tracing::field::Visit for Points {
            fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
                if field.name() == "points" {
                    self.0 = value;
                }
            }

            fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
                if field.name() == "points" {
                    self.0 = value.try_into().expect("a point count is not negative");
                }
            }

            fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
        }

        let mut points = Points(0);
        event.record(&mut points);
        let points = usize::try_from(points.0).expect("a point count fits a usize");
        self.0.fetch_add(points, Ordering::Relaxed);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

/// Converts `metrics`, and counts the points the warnings report as dropped.
fn convert_counting_drops(scenario: &Scenario, metrics: &ResourceMetrics) -> (String, usize) {
    let dropped = Arc::new(AtomicUsize::new(0));
    let text = tracing::subscriber::with_default(DroppedPoints(Arc::clone(&dropped)), || {
        convert(scenario, metrics)
    });
    (text, dropped.load(Ordering::Relaxed))
}

/// Removes one copy of each item of `written` from `recorded`. Panics with
/// `context` if an item was not recorded, or was written more often.
fn remove_written<T: PartialEq + std::fmt::Debug>(
    recorded: &mut Vec<T>,
    written: &[T],
    context: &str,
) {
    for item in written {
        let index = recorded.iter().position(|r| r == item).unwrap_or_else(|| {
            panic!("{item:?} was written but not recorded, or written twice\n{context}")
        });
        recorded.swap_remove(index);
    }
}

#[ephyra::test(gen = scenario(), tests = 2000)]
fn every_data_point_is_written_once(scenario: Scenario) {
    let metrics = sdk::collect(&scenario, Order::AsWritten);
    let (text, dropped) = convert_counting_drops(&scenario, &metrics);
    let lines =
        exposition::parse(&text).unwrap_or_else(|e| panic!("unreadable output: {e}\n{text}"));
    let (mut recorded, mut descriptions) = expected_points(&scenario, &metrics);

    let mut written = Vec::new();
    for FamilySample {
        family,
        typ,
        sample,
    } in exposition::samples_by_family(&lines)
    {
        let Some(role) = value_role(family, typ, sample) else {
            continue;
        };
        let value = Number::parse(&sample.value)
            .unwrap_or_else(|| panic!("unreadable value {:?}\n{text}", sample.value));
        let mut values: Vec<String> = sample
            .labels
            .iter()
            .filter(|(name, _)| role != "histogram" || name != "le")
            .map(|(_, value)| value.clone())
            .collect();
        values.sort();
        written.push((role, values, value));
    }
    // om[verify escaping.chars]
    // om[verify numbers.float]
    // om[verify numbers.integer]
    // c[verify mattrs.to-labels]
    remove_written(&mut recorded, &written, &text);
    // c[verify metadata.drop-warn] - every point that is not written is reported
    assert_eq!(
        recorded.len(),
        dropped,
        "points neither written nor reported as dropped: {recorded:?}\n{text}"
    );

    let helps: Vec<String> = lines
        .iter()
        .filter_map(|l| match l {
            Line::Help { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    // c[verify metadata.help-description]
    // c[verify metadata.conflict-unit-help] - a family keeps one of its descriptions
    // om[verify escaping.chars]
    remove_written(&mut descriptions, &helps, &text);
}

#[ephyra::test(gen = scenario(), tests = 2000)]
fn histogram_points_are_consistent(scenario: Scenario) {
    let text = convert(&scenario, &sdk::collect(&scenario, Order::AsWritten));
    let lines =
        exposition::parse(&text).unwrap_or_else(|e| panic!("unreadable output: {e}\n{text}"));

    // Buckets of one histogram point, keyed by family and labels without `le`.
    let mut buckets: BTreeMap<(String, Vec<(String, String)>), Vec<(f64, Number)>> =
        BTreeMap::new();
    let mut counts = HashMap::new();
    let mut sums = HashSet::new();
    for FamilySample {
        family,
        typ,
        sample,
    } in exposition::samples_by_family(&lines)
    {
        if typ != "histogram" {
            continue;
        }
        let mut labels: Vec<_> = sample
            .labels
            .iter()
            .filter(|(n, _)| n != "le")
            .cloned()
            .collect();
        labels.sort();
        let key = (family.to_owned(), labels);
        if sample.name == format!("{family}_bucket") {
            let le = sample
                .label("le")
                .unwrap_or_else(|| panic!("bucket without `le`\n{text}"));
            let bound = match Number::parse(le) {
                Some(Number::Float(bits)) => f64::from_bits(bits),
                other => panic!("`le` is not a float: {le:?} ({other:?})\n{text}"),
            };
            buckets
                .entry(key)
                .or_default()
                .push((bound, Number::parse(&sample.value).unwrap()));
        } else if sample.name == format!("{family}_count") {
            counts.insert(key, Number::parse(&sample.value).unwrap());
        } else if sample.name == format!("{family}_sum") {
            sums.insert(key);
        }
    }
    for (key, points) in &buckets {
        // om[verify histogram.count-iff-sum]
        assert_eq!(
            sums.contains(key),
            counts.contains_key(key),
            "_sum and _count disagree for {key:?}\n{text}"
        );
        // om[verify histogram.negative-threshold-nosum]
        // c[verify histogram.sum] - "The sum is positive and monotonic when all buckets are positive."
        if points.iter().any(|(bound, _)| *bound < 0.0) {
            assert!(
                !sums.contains(key),
                "_sum next to a negative bucket threshold for {key:?}\n{text}"
            );
        }
        // om[verify histogram.inf-bucket]
        assert_eq!(
            points.last().map(|p| p.0),
            Some(f64::INFINITY),
            "no +Inf bucket for {key:?}\n{text}"
        );
        for pair in points.windows(2) {
            // om[verify histogram.sorted]
            assert!(
                pair[0].0 < pair[1].0,
                "buckets out of order for {key:?}\n{text}"
            );
            // c[verify histogram.bucket.cumulative]
            assert!(
                pair[0].1 <= pair[1].1,
                "bucket counts decrease for {key:?}\n{text}"
            );
        }
        // +Inf holds every observation.
        if let Some(count) = counts.get(key) {
            assert_eq!(
                &points.last().unwrap().1,
                count,
                "+Inf bucket differs from _count for {key:?}\n{text}"
            );
        }
    }
}

/// Removes everything that legitimately differs between two collections of
/// the same measurements: timestamps, and `_created` values, which are timestamps.
fn without_times(lines: &[Line]) -> Vec<Line> {
    let mut family = String::new();
    lines
        .iter()
        .map(|line| match line {
            Line::Type { family: f, .. } => {
                family = f.clone();
                line.clone()
            }
            Line::Sample(sample) => {
                let mut sample = sample.clone();
                sample.timestamp = None;
                if let Some(exemplar) = &mut sample.exemplar {
                    exemplar.timestamp = None;
                }
                if sample.name == format!("{family}_created") {
                    sample.value.clear();
                }
                Line::Sample(sample)
            }
            other => other.clone(),
        })
        .collect()
}

#[ephyra::test(gen = scenario(), tests = 2000)]
fn output_does_not_depend_on_recording_order(scenario: Scenario) {
    let forward = convert(&scenario, &sdk::collect(&scenario, Order::AsWritten));
    let reversed = convert(&scenario, &sdk::collect(&scenario, Order::Reversed));
    let forward_lines =
        exposition::parse(&forward).unwrap_or_else(|e| panic!("unreadable output: {e}\n{forward}"));
    let reversed_lines = exposition::parse(&reversed)
        .unwrap_or_else(|e| panic!("unreadable output: {e}\n{reversed}"));
    // Two families of one name have no defined order; `output_is_well_formed`
    // reports them, so name that case apart from other ordering failures.
    let mut families = HashSet::new();
    let repeated_family = forward_lines.iter().any(|line| match line {
        Line::Type { family, .. } => !families.insert(family),
        _ => false,
    });
    if repeated_family {
        ephyra::signature("order differs, with a repeated family name");
    }
    // The converter sorts scopes, metrics and points, and attribute order is
    // not part of a label set, so the order of recording cannot matter.
    assert_eq!(
        without_times(&forward_lines),
        without_times(&reversed_lines),
        "\n{forward}\n---\n{reversed}"
    );
}
