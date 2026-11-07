//! Minimal inputs for the bugs the properties in `properties.rs` found. Each
//! test fails on ottotom 0.33.0 and cites the sentence it checks.

use std::collections::HashSet;

use ottotom::convert::WriteOpenMetrics;
use ottotom_ephyra_tests::exposition::{self, Line, Sample};
use ottotom_ephyra_tests::scenario::{Attr, AttrValue, Instrument, Kind, Record, Scenario, Scope};
use ottotom_ephyra_tests::sdk::{self, Order};

fn attr(key: &str, value: &str) -> Attr {
    Attr {
        key: key.into(),
        value: AttrValue::Str(value.into()),
    }
}

fn record(int: i64, float: f64, attrs: Vec<Attr>) -> Record {
    Record { int, float, attrs }
}

fn instrument(name: &str, kind: Kind, records: Vec<Record>) -> Instrument {
    Instrument {
        name: name.into(),
        unit: String::new(),
        description: String::new(),
        kind,
        bounds: None,
        records,
    }
}

fn scope(name: &str, instruments: Vec<Instrument>) -> Scope {
    Scope {
        name: name.into(),
        version: None,
        attrs: Vec::new(),
        instruments,
    }
}

/// A scenario without `target_info` and `otel_scope_info`, so the output
/// holds only what the test is about.
fn plain(scopes: Vec<Scope>) -> Scenario {
    Scenario {
        resource: Vec::new(),
        scopes,
        scope_info: false,
        target_info: false,
    }
}

fn render(scenario: &Scenario, order: Order) -> String {
    sdk::collect(scenario, order)
        .to_openmetrics_string_with_config(sdk::config(scenario))
        .unwrap()
}

fn samples(text: &str) -> Vec<Sample> {
    exposition::parse(text)
        .unwrap()
        .into_iter()
        .filter_map(|line| match line {
            Line::Sample(sample) => Some(sample),
            _ => None,
        })
        .collect()
}

fn type_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with("# TYPE "))
        .map(str::to_owned)
        .collect()
}

#[test]
// c[verify mattrs.key-sanitize.collisions] - "If multiple key-value pairs are converted
// to have the same Prometheus key, the values MUST be concatenated together,
// separated by `;`, and ordered by the lexicographical order of the original keys."
fn attribute_keys_that_sanitize_alike_are_concatenated() {
    let scenario = plain(vec![scope(
        "s",
        vec![instrument(
            "g",
            Kind::U64Gauge,
            vec![record(1, 0.0, vec![attr("a_b", "y"), attr("a.b", "x")])],
        )],
    )]);
    let text = render(&scenario, Order::AsWritten);
    // Currently: g{a_b="x",a_b="y"} 1 ...
    assert_eq!(
        samples(&text)[0].labels,
        vec![("a_b".to_owned(), "x;y".to_owned())],
        "\n{text}"
    );
}

#[test]
// om[verify labels.unique] - "Label names MUST be unique within a LabelSet."
fn attribute_named_otel_scope_name_does_not_repeat_the_label() {
    let mut scenario = plain(vec![scope(
        "s",
        vec![instrument(
            "g",
            Kind::U64Gauge,
            vec![record(1, 0.0, vec![attr("otel_scope_name", "x")])],
        )],
    )]);
    scenario.scope_info = true;
    let text = render(&scenario, Order::AsWritten);
    for sample in samples(&text) {
        let names: HashSet<_> = sample.labels.iter().map(|(n, _)| n).collect();
        // Currently: g{otel_scope_name="x",otel_scope_name="s"} 1 ...
        assert_eq!(
            names.len(),
            sample.labels.len(),
            "{:?}\n{text}",
            sample.labels
        );
    }
}

#[test]
// om[verify histogram.nole] - "A Histogram's Metric's LabelSet MUST NOT have a `le` label name."
fn histogram_attribute_named_le_does_not_become_a_label() {
    let scenario = plain(vec![scope(
        "s",
        vec![instrument(
            "h",
            Kind::U64Histogram,
            vec![record(1, 0.0, vec![attr("le", "x")])],
        )],
    )]);
    let text = render(&scenario, Order::AsWritten);
    for sample in samples(&text) {
        let le_labels = sample.labels.iter().filter(|(n, _)| n == "le").count();
        let expected = usize::from(sample.name == "h_bucket");
        // Currently: h_count{le="x"} 1 ..., h_bucket{le="x",le="0"} 0 ...
        assert_eq!(
            le_labels, expected,
            "{}{:?}\n{text}",
            sample.name, sample.labels
        );
    }
}

#[test]
// c[verify metadata.no-duplicates] - "Prometheus Pull exporters MUST NOT allow duplicate
// UNIT, HELP, or TYPE comments for the same metric name in a single scrape."
// om[verify metricfamily.nointerleave] - "MetricFamilies MUST NOT be interleaved."
fn one_metric_name_in_two_scopes_is_one_family() {
    let mut scenario = plain(vec![
        scope(
            "lib.one",
            vec![instrument(
                "requests",
                Kind::U64Counter,
                vec![record(1, 0.0, vec![])],
            )],
        ),
        scope(
            "lib.two",
            vec![instrument(
                "requests",
                Kind::U64Counter,
                vec![record(2, 0.0, vec![])],
            )],
        ),
    ]);
    scenario.scope_info = true;
    let text = render(&scenario, Order::AsWritten);
    // Currently: `# TYPE requests counter` twice, one family per scope.
    assert_eq!(
        type_lines(&text)
            .iter()
            .filter(|l| l.starts_with("# TYPE requests "))
            .count(),
        1,
        "\n{text}"
    );
}

#[test]
// c[verify metadata.no-duplicates] - as above; the SDK keeps a u64 and an f64
// counter of one name as two metrics.
fn one_metric_name_with_two_number_types_is_one_family() {
    let scenario = plain(vec![scope(
        "s",
        vec![
            instrument("a", Kind::U64Counter, vec![record(1, 0.0, vec![])]),
            instrument("a", Kind::F64Counter, vec![record(0, 1.5, vec![])]),
        ],
    )]);
    let text = render(&scenario, Order::AsWritten);
    assert_eq!(type_lines(&text), vec!["# TYPE a counter"], "\n{text}");
}

#[test]
// om[verify metricfamily.name-clash] - "The name of a MetricFamily MUST NOT result in a
// potential clash for sample metric names with another MetricFamily."
fn counter_and_gauge_do_not_share_a_sample_name() {
    let scenario = plain(vec![scope(
        "s",
        vec![
            instrument("a", Kind::U64Counter, vec![record(1, 0.0, vec![])]),
            instrument("a_total", Kind::U64Gauge, vec![record(2, 0.0, vec![])]),
        ],
    )]);
    let text = render(&scenario, Order::AsWritten);
    // Currently: family `a` writes `a_total 1`, family `a_total` writes `a_total 2`.
    let named_a_total = samples(&text)
        .iter()
        .filter(|s| s.name == "a_total")
        .count();
    assert!(named_a_total <= 1, "\n{text}");
}

#[test]
// om[verify histogram.count-iff-sum] - "If and only if a Sum Value is present in a
// MetricPoint, then the MetricPoint's +Inf Bucket value MUST also appear in a
// Sample with a MetricName with the suffix `_count`."
fn histogram_without_sum_has_no_count() {
    let scenario = plain(vec![scope(
        "s",
        vec![instrument(
            "h",
            Kind::F64Histogram,
            vec![record(0, -1.0, vec![])],
        )],
    )]);
    let text = render(&scenario, Order::AsWritten);
    let names: Vec<String> = samples(&text).into_iter().map(|s| s.name).collect();
    assert!(!names.contains(&"h_sum".to_owned()), "\n{text}");
    // Currently: h_count 1 ... without h_sum.
    assert!(!names.contains(&"h_count".to_owned()), "\n{text}");
}

#[test]
// om[verify histogram.negative-threshold-nosum] - "Negative threshold buckets
// MAY be used, but then the Histogram MetricPoint MUST NOT contain a sum value."
// c[verify histogram.sum] - "The sum is positive and monotonic when all buckets are positive."
fn histogram_with_negative_bucket_threshold_has_no_sum() {
    let mut h = instrument("h", Kind::F64Histogram, vec![record(0, 0.5, vec![])]);
    h.bounds = Some(vec![-1.0, 1.0]);
    let scenario = plain(vec![scope("s", vec![h])]);
    let text = render(&scenario, Order::AsWritten);
    // Currently: h_sum 0.5 ... next to h_bucket{le="-1.0"} 0 ...
    assert!(samples(&text).iter().all(|s| s.name != "h_sum"), "\n{text}");
}

#[test]
// Deterministic output: scopes, metrics and points are sorted.
fn family_order_does_not_depend_on_instrument_creation_order() {
    let mut ratio = instrument("b", Kind::U64Counter, vec![record(1, 0.0, vec![])]);
    ratio.unit = "1".into();
    let scenario = plain(vec![scope(
        "s",
        vec![
            instrument("b", Kind::U64Gauge, vec![record(2, 0.0, vec![])]),
            ratio,
        ],
    )]);
    let forward = type_lines(&render(&scenario, Order::AsWritten));
    let reversed = type_lines(&render(&scenario, Order::Reversed));
    // Currently: `b` then `b_ratio` in one order, `b_ratio` then `b` in the other.
    assert_eq!(forward, reversed);
}
