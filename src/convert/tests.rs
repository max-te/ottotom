use std::time::{Duration, SystemTime, UNIX_EPOCH};

use insta::assert_snapshot;
use opentelemetry::{Array, InstrumentationScope, KeyValue, StringValue, Value};
use opentelemetry_sdk::metrics::data::{MetricData, ScopeMetrics};
use ottotom_testsupport::metric_data::{
    FromAggregated, make_f64_counter_metric, make_f64_exponential_histogram_metric_handle,
    make_f64_gauge_metric, make_f64_histogram_metric, make_i64_counter_metric,
    make_i64_gauge_metric, make_u64_counter_metric_handle, make_u64_gauge_metric,
    make_u64_gauge_metric_handle, make_u64_histogram_metric,
};
use ottotom_testsupport::resource_metrics::{
    collect_from_scopes, collect_metrics, collect_metrics_with_temporality, make_test_metrics,
};
use ottotom_testsupport::timestamps::ExtractTimestamps;
use ufmt::uwrite;

use super::exemplar::{ExemplarPoint, falls_in_bucket};
use super::label_sets::sort_by_label_set;
use super::labels::{write_attrs, write_escaped_value};
use super::*;

fn strip_otel_scope_name(s: &str) -> String {
    const OTEL_SCOPE_NAME: &str = "otel_scope_name=\"myscope\"";

    let mut result = s.to_owned();
    while let Some(start) = result.find(OTEL_SCOPE_NAME) {
        result.replace_range(start..start + OTEL_SCOPE_NAME.len(), "");
        if result.as_bytes()[start - 1..start] == *b"," {
            // Had preceding attributes
            result.replace_range(start - 1..start, "");
        } else if result.as_bytes()[start..=start] == *b"," {
            // Was first attribute with trailing attributes
            result.replace_range(start..=start, "");
        }
    }

    result
}

#[test]
// c[verify metadata.name-sanitize]
fn test_write_sanitized_metric_name() {
    let mut output = String::new();

    // Test with valid name
    write_sanitized_name(&mut output, "valid_metric_name", NameKind::Metric).unwrap();
    assert_eq!(output, "valid_metric_name");

    // Test with name containing invalid characters
    output.clear();
    write_sanitized_name(&mut output, "invalid._ä.metric-name", NameKind::Metric).unwrap();
    assert_eq!(output, "invalid_metric_name");

    // Test with name starting with digit
    output.clear();
    write_sanitized_name(&mut output, "1.metric", NameKind::Metric).unwrap();
    assert_eq!(output, "_1_metric");

    // Multiple consecutive invalid chars should collapse to single underscore
    output.clear();
    write_sanitized_name(&mut output, "a..b", NameKind::Metric).unwrap();
    assert_eq!(output, "a_b");

    // Mixed invalid chars should collapse
    output.clear();
    write_sanitized_name(&mut output, "a..-..b", NameKind::Metric).unwrap();
    assert_eq!(output, "a_b");

    // Leading invalid char becomes underscore
    output.clear();
    write_sanitized_name(&mut output, ".abc", NameKind::Metric).unwrap();
    assert_eq!(output, "_abc");

    // Colons are allowed
    output.clear();
    write_sanitized_name(&mut output, "my:metric:name", NameKind::Metric).unwrap();
    assert_eq!(output, "my:metric:name");

    // Colons mixed with invalid chars
    output.clear();
    write_sanitized_name(&mut output, "my:metric.name", NameKind::Metric).unwrap();
    assert_eq!(output, "my:metric_name");
}

#[test]
// c[verify mattrs.key-sanitize]
fn test_write_sanitized_attribute_label_name() {
    let mut output = String::new();

    // Test with valid label name
    write_sanitized_name(&mut output, "valid_label_name", NameKind::AttributeLabel).unwrap();
    assert_eq!(output, "valid_label_name");

    // Test with label name containing invalid characters
    output.clear();
    write_sanitized_name(
        &mut output,
        "invalid._ä.label-name",
        NameKind::AttributeLabel,
    )
    .unwrap();
    assert_eq!(output, "invalid_label_name");

    // Test with label name starting with digit
    output.clear();
    write_sanitized_name(&mut output, "1.label", NameKind::AttributeLabel).unwrap();
    assert_eq!(output, "_1_label");

    // Multiple consecutive invalid chars should collapse to single underscore
    output.clear();
    write_sanitized_name(&mut output, "a..b", NameKind::AttributeLabel).unwrap();
    assert_eq!(output, "a_b");

    // Mixed invalid chars should collapse
    output.clear();
    write_sanitized_name(&mut output, "a..-..b", NameKind::AttributeLabel).unwrap();
    assert_eq!(output, "a_b");

    // Leading invalid char becomes underscore
    output.clear();
    write_sanitized_name(&mut output, ".abc", NameKind::AttributeLabel).unwrap();
    assert_eq!(output, "_abc");

    // Colons are NOT allowed in labels (unlike metric names)
    output.clear();
    write_sanitized_name(&mut output, "my:label:name", NameKind::AttributeLabel).unwrap();
    assert_eq!(output, "my_label_name");
}

#[test]
// om[verify escaping.chars]
// om[verify strings.utf8]
fn test_write_escaped() {
    let mut output = String::new();

    // Test with string containing characters that need escaping
    write_escaped(
        &mut output,
        "Line 1\nLine 2\tTabbed\r\nWindows \"quoted\" \\ BS ❤️‍🩹",
    )
    .unwrap();
    assert_eq!(
        output,
        "Line 1\\nLine 2\tTabbed\r\\nWindows \\\"quoted\\\" \\\\ BS ❤️‍🩹"
    );

    // Test with string not needing escaping
    output.clear();
    write_escaped(&mut output, "Simple string").unwrap();
    assert_eq!(output, "Simple string");
}

#[test]
fn test_hash_attrs() {
    let attrs = [
        KeyValue::new("key1", "value1"),
        KeyValue::new("key2", "value2"),
    ];

    let hash1 = hash_attrs(attrs.iter());

    // Same attributes should produce same hash, order does not matter
    let attrs2 = [
        KeyValue::new("key2", "value2"),
        KeyValue::new("key1", "value1"),
    ];
    let hash2 = hash_attrs(attrs2.iter());

    assert_eq!(hash1, hash2);

    // Different attributes should produce different hash
    let attrs3 = [
        KeyValue::new("key1", "value1"),
        KeyValue::new("key2", "different"),
    ];
    let hash3 = hash_attrs(attrs3.iter());

    assert_ne!(hash1, hash3);
}

#[test]
// c[verify mattrs.to-labels]
fn test_write_attrs() {
    let mut output = String::new();
    let attrs = [
        KeyValue::new("key1", "value1"),
        KeyValue::new("key2", "value2"),
    ];

    write_attrs(&mut output, attrs.iter()).unwrap();
    assert_eq!(output, "key1=\"value1\",key2=\"value2\"");

    // Test with attributes containing characters that need escaping
    output.clear();
    let attrs_with_escapes = [
        KeyValue::new("key1", "value\nwith\nnewlines"),
        KeyValue::new("key2", "value\"with\"quotes"),
    ];

    write_attrs(&mut output, attrs_with_escapes.iter()).unwrap();
    assert_eq!(
        output,
        "key1=\"value\\nwith\\nnewlines\",key2=\"value\\\"with\\\"quotes\""
    );
}

#[test]
// c[verify mattrs.key-sanitize.collisions]
fn test_write_attrs_concatenates_colliding_labels() {
    let mut output = String::new();
    let attrs = [
        KeyValue::new("c", "\""),
        KeyValue::new("a_b", "y"),
        KeyValue::new("a.b", "x"),
    ];

    write_attrs(&mut output, attrs.iter()).unwrap();
    // `a.b` sorts before `a_b`, so its value comes first.
    assert_eq!(output, "a_b=\"x;y\",c=\"\\\"\"");
}

#[test]
// c[verify attrs.stringify]
fn test_write_attrs_stringify() {
    let mut output = String::new();

    // int64(100) -> "100"
    write_attrs(&mut output, [KeyValue::new("int", Value::I64(100))].iter()).unwrap();
    assert_eq!(output, "int=\"100\"");

    // float64(1.5) -> "1.5"
    output.clear();
    write_attrs(
        &mut output,
        [KeyValue::new("float", Value::F64(1.5))].iter(),
    )
    .unwrap();
    assert_eq!(output, "float=\"1.5\"");

    // bool -> "true"/"false"
    output.clear();
    write_attrs(
        &mut output,
        [KeyValue::new("bool", Value::Bool(true))].iter(),
    )
    .unwrap();
    assert_eq!(output, "bool=\"true\"");
    output.clear();
    write_attrs(
        &mut output,
        [KeyValue::new("bool", Value::Bool(false))].iter(),
    )
    .unwrap();
    assert_eq!(output, "bool=\"false\"");

    // strings pass through unchanged
    output.clear();
    write_attrs(&mut output, [KeyValue::new("str", "he\u{0000}llo")].iter()).unwrap();
    assert_eq!(output, "str=\"he\u{0000}llo\"");

    // empty array of any type -> "[]"
    output.clear();
    write_attrs(
        &mut output,
        [KeyValue::new("arr", Value::Array(Array::I64(vec![])))].iter(),
    )
    .unwrap();
    assert_eq!(output, "arr=\"[]\"");

    // non-empty arrays are JSON-encoded
    output.clear();
    write_attrs(
        &mut output,
        [KeyValue::new(
            "arr",
            Value::Array(Array::I64(vec![1, 2, 3])),
        )]
        .iter(),
    )
    .unwrap();
    assert_eq!(output, "arr=\"[1,2,3]\"");

    output.clear();
    write_attrs(
        &mut output,
        [KeyValue::new(
            "arr",
            Value::Array(Array::F64(vec![1.5, 2.5])),
        )]
        .iter(),
    )
    .unwrap();
    assert_eq!(output, "arr=\"[1.5,2.5]\"");

    output.clear();
    write_attrs(
        &mut output,
        [KeyValue::new(
            "arr",
            Value::Array(Array::Bool(vec![true, false])),
        )]
        .iter(),
    )
    .unwrap();
    assert_eq!(output, "arr=\"[true,false]\"");

    output.clear();
    write_attrs(
        &mut output,
        [KeyValue::new(
            "arr",
            Value::Array(Array::String(vec!["a".into(), "b".into()])),
        )]
        .iter(),
    )
    .unwrap();
    assert_eq!(output, r#"arr="[\"a\",\"b\"]""#);
}

#[test]
// c[verify scope.config-disable] - behavior differs with/without the config
// c[verify scope.name-version] - scope name and version become info attributes
fn test_make_scope_name_attrs() {
    let scope_name = "test_scope";
    let attrs = make_scope_name_attrs(Config::default(), scope_name, None);
    assert_eq!(attrs.len(), 1);
    assert_eq!(attrs[0].key.as_str(), "otel_scope_name");
    assert_eq!(attrs[0].value.as_str(), "test_scope");

    let scope_version = "1.2.3";
    let attrs = make_scope_name_attrs(Config::default(), scope_name, Some(scope_version));
    assert_eq!(attrs.len(), 2);
    assert_eq!(attrs[0].key.as_str(), "otel_scope_name");
    assert_eq!(attrs[0].value.as_str(), "test_scope");
    assert_eq!(attrs[1].key.as_str(), "otel_scope_version");
    assert_eq!(attrs[1].value.as_str(), "1.2.3");

    let disabled = Config::builder().scope_info_enabled(false).build();
    let attrs = make_scope_name_attrs(disabled, scope_name, None);
    assert_eq!(attrs, [] as [KeyValue; 0]);
    let attrs = make_scope_name_attrs(disabled, scope_name, Some(scope_version));
    assert_eq!(attrs, [] as [KeyValue; 0]);
}

#[test]
// om[verify timestamp.unix]
fn test_to_timestamp() {
    use std::time::{Duration, UNIX_EPOCH};

    // Test with a known timestamp
    let time = UNIX_EPOCH + Duration::from_secs(1_625_097_600);
    let timestamp = to_timestamp(time);
    let mut output = String::new();
    uwrite!(output, "{}", timestamp).unwrap();
    assert_eq!(output, "1625097600.0");
}

#[test]
fn test_write_otel_scope_info() {
    let resource_metrics = make_test_metrics();
    let scopes: Vec<&ScopeMetrics> = resource_metrics.scope_metrics().collect();

    let mut output = String::new();
    write_otel_scope_info(&mut output, &scopes, Config::default()).unwrap();

    // c[verify scope.info]
    assert!(output.contains("# TYPE otel_scope info"));
    assert!(output.contains("otel_scope_info{"));
    // c[verify scope.name-version]
    assert!(output.contains("otel_scope_name=\"meter.1\""));
    assert!(output.contains("otel_scope_version="));
    // c[verify scope.attribute-labels] - scope attributes become labels
    assert!(output.contains("scopek=\"scopev\""));
}

#[test]
fn test_get_type() {
    let resource_metrics = make_test_metrics();
    let scopes: Vec<&ScopeMetrics> = resource_metrics.scope_metrics().collect();

    for scope in scopes {
        for metric in scope.metrics() {
            let result = get_type(metric.data());
            assert!(result.is_ok());

            // Check that the type is one of the expected values
            let type_str = result.unwrap();
            assert!(
                type_str == "gauge" || type_str == "counter" || type_str == "histogram",
                "Unexpected metric type: {type_str}"
            );
        }
    }
}

#[test]
fn test_write_gauge() {
    let metric = make_f64_gauge_metric(vec![
        (4.2, vec![KeyValue::new("kk", "v1")]),
        (4.23, vec![KeyValue::new("kk", "v2")]),
    ]);
    let ts = metric
        .time()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        .to_string();

    let mut output = String::new();

    let mut ctx = Context {
        attr_buffer: String::from("staledata"),
        scope_name: "myscope",
        ..Context::with_output(&mut output)
    };

    write_gauge(&mut ctx.family("mygauge", "gauge", None), &metric).unwrap();
    let output = output.replace(&ts, "<TIMESTAMP>");
    assert_snapshot!(strip_otel_scope_name(&output));
}

#[test]
// c[verify sum.cumulative-monotonic]
// c[verify sum.total-suffix]
// c[verify sum.created]
fn test_write_counter() {
    let values = vec![(125, vec![KeyValue::new("kk", "v1")])];

    // Build a counter family by name, write its samples, and mask the volatile
    // timestamps. The name (via `extract_type_unit_and_name`) and the data (via
    // `write_counter`) both come from the same instrument.
    let render = |name: &'static str| {
        let handle = make_u64_counter_metric_handle(name, None, None, values.clone());
        let sum = handle.extract::<Sum<u64>>().unwrap();
        let ts = sum
            .time()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            .to_string();
        let created = sum
            .start_time()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            .to_string();

        let mut output = String::new();
        let mut ctx = Context {
            attr_buffer: String::from("staledata"),
            scope_name: "myscope",
            ..Context::with_output(&mut output)
        };
        let mut name = String::new();
        let (typ, unit) = extract_type_unit_and_name(&mut name, &handle).unwrap();
        write_counter(&mut ctx.family(&name, typ, unit.as_deref()), &sum).unwrap();

        output
            .replace(&ts, "<TIMESTAMP>")
            .replace(&created, "<CREATED>")
    };

    let output = render("mycounter");
    assert_snapshot!(strip_otel_scope_name(&output));

    // c[verify sum.total-suffix] - a pre-existing `_total` in the metric name is
    // stripped from the MetricFamily name and re-appended to the value sample,
    // so `mycounter` and `mycounter_total` converge on the same family.
    assert_eq!(
        strip_otel_scope_name(&output),
        strip_otel_scope_name(&render("mycounter_total"))
    );
}

/// A stand-in for the SDK's [`Exemplar`], which has no public constructor.
struct TestExemplar<T> {
    value: T,
    time: SystemTime,
    filtered_attributes: Vec<KeyValue>,
    span_id: [u8; 8],
    trace_id: [u8; 16],
}

impl<T: Numeric + Copy> ExemplarPoint for TestExemplar<T> {
    type Value = T;

    fn value(&self) -> T {
        self.value
    }

    fn time(&self) -> SystemTime {
        self.time
    }

    fn trace_id(&self) -> [u8; 16] {
        self.trace_id
    }

    fn span_id(&self) -> [u8; 8] {
        self.span_id
    }

    fn filtered_attributes(&self) -> impl Iterator<Item = &KeyValue> {
        self.filtered_attributes.iter()
    }
}

/// An exemplar carrying nothing but a value, recorded `millis` after the epoch.
fn bare_exemplar<T>(value: T, millis: u64) -> TestExemplar<T> {
    TestExemplar {
        value,
        time: UNIX_EPOCH + Duration::from_millis(millis),
        filtered_attributes: vec![],
        span_id: [0; 8],
        trace_id: [0; 16],
    }
}

#[test]
// om[verify exemplars.structure]
// om[verify exemplars.empty-labelset]
// c[verify exemplar.trace-span-ids]
// c[verify exemplar.filtered-attrs]
// c[verify exemplar.timestamp]
fn test_write_exemplar() {
    let exemplar = TestExemplar {
        value: 0.67,
        time: UNIX_EPOCH + Duration::from_millis(123_456),
        filtered_attributes: vec![KeyValue::new("filtered", "yes")],
        span_id: [0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0xba, 0xbe],
        trace_id: [
            0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab,
            0xcd, 0xef,
        ],
    };
    let mut output = String::new();
    write_exemplar(&mut output, std::iter::once(&exemplar)).unwrap();
    assert_eq!(
        output,
        " # {filtered=\"yes\",span_id=\"deadbeefcafebabe\",trace_id=\"1234567890abcdef1234567890abcdef\"} 0.67 123.456"
    );

    // Zeroed ids and no filtered attributes render as an empty label set.
    let mut output = String::new();
    write_exemplar(&mut output, std::iter::once(&bare_exemplar(1.0, 0))).unwrap();
    assert_eq!(output, " # {} 1.0 0.0");
}

#[test]
// om[verify exemplars.bucket-attachment]
// c[verify exemplar.bucket-single] - one exemplar per bucket, the latest
fn test_write_bucket_exemplar() {
    const BOUNDS: [f64; 3] = [1.0, 2.0, 3.0];
    let exemplars = [
        bare_exemplar(0.5, 1),
        bare_exemplar(1.0, 2),
        bare_exemplar(2.5, 3),
        bare_exemplar(9.0, 4),
    ];

    let render = |bucket| {
        let mut output = String::new();
        write_bucket_exemplar(&mut output, &BOUNDS, bucket, &exemplars).unwrap();
        output
    };

    // Both 0.5 and 1.0 fall in the first bucket; the later one wins.
    assert_eq!(render(0), " # {} 1.0 0.002");
    // Nothing lies in (1, 2].
    assert_eq!(render(1), "");
    assert_eq!(render(2), " # {} 2.5 0.003");
    // The index past the last bound is the +Inf bucket.
    assert_eq!(render(3), " # {} 9.0 0.004");
}

#[test]
// c[verify histogram.sum] - a histogram that saw a negative observation has no
// monotonic sum, so `_sum` is omitted.
fn test_write_histogram_omits_sum_when_not_monotonic() {
    let negative = make_f64_histogram_metric(vec![(-1.0, vec![])]);
    let mut output = String::new();
    write_histogram(
        &mut Context::with_output(&mut output).family("", "histogram", None),
        &negative,
    )
    .unwrap();
    assert!(!output.contains("_sum"), "{output}");

    let nonnegative = make_f64_histogram_metric(vec![(1.0, vec![])]);
    let mut output = String::new();
    write_histogram(
        &mut Context::with_output(&mut output).family("", "histogram", None),
        &nonnegative,
    )
    .unwrap();
    assert!(output.contains("_sum"), "{output}");

    // Unsigned values need no such check; their sum is monotonic by type.
    let unsigned = make_u64_histogram_metric(vec![(1, vec![])]);
    let mut output = String::new();
    write_histogram(
        &mut Context::with_output(&mut output).family("", "histogram", None),
        &unsigned,
    )
    .unwrap();
    assert!(output.contains("_sum"), "{output}");
}

#[test]
// om[verify histogram.count-iff-sum]
fn test_write_histogram_omits_count_without_sum() {
    let negative = make_f64_histogram_metric(vec![(-1.0, vec![])]);
    let mut output = String::new();
    write_histogram(
        &mut Context::with_output(&mut output).family("", "histogram", None),
        &negative,
    )
    .unwrap();
    assert!(!output.contains("_count"), "{output}");
    // The +Inf bucket still carries the count.
    assert!(output.contains("le=\"+Inf\"} 1 "), "{output}");

    let nonnegative = make_f64_histogram_metric(vec![(1.0, vec![])]);
    let mut output = String::new();
    write_histogram(
        &mut Context::with_output(&mut output).family("", "histogram", None),
        &nonnegative,
    )
    .unwrap();
    assert!(output.contains("_count"), "{output}");
}

#[test]
// om[verify histogram.nole]
fn test_write_histogram_drops_points_with_le_attribute() {
    let histogram = make_u64_histogram_metric(vec![
        (1, vec![KeyValue::new("le", "x")]),
        (2, vec![KeyValue::new("k", "v")]),
    ]);
    let mut output = String::new();
    write_histogram(
        &mut Context::with_output(&mut output).family("", "histogram", None),
        &histogram,
    )
    .unwrap();
    assert!(!output.contains("le=\"x\""), "{output}");
    assert!(output.contains("k=\"v\""), "{output}");
}

#[test]
// om[verify labels.unique]
fn test_write_gauge_drops_points_shadowing_scope_labels() {
    let gauge = make_f64_gauge_metric(vec![
        (1.0, vec![KeyValue::new("otel.scope.name", "x")]),
        (2.0, vec![KeyValue::new("k", "v")]),
    ]);
    let mut output = String::new();
    write_gauge(
        &mut Context::with_output(&mut output).family("", "gauge", None),
        &gauge,
    )
    .unwrap();
    assert!(!output.contains("\"x\""), "{output}");
    assert!(output.contains("k=\"v\""), "{output}");

    // Without scope labels, the attribute is an ordinary label.
    let config = Config::builder().scope_info_enabled(false).build();
    let mut output = String::new();
    write_gauge(
        &mut Context::with_config(&mut output, config).family("", "gauge", None),
        &gauge,
    )
    .unwrap();
    assert!(output.contains("otel_scope_name=\"x\""), "{output}");
}

#[test]
fn test_sanitizes_to() {
    for key in ["otel_scope_name", "otel.scope.name", "otel__scope-name"] {
        assert!(sanitizes_to(key, "otel_scope_name"), "{key}");
    }
    for key in [
        "otel.scope.names",
        "otel_scope",
        "_otel_scope_name",
        "Otel_scope_name",
    ] {
        assert!(!sanitizes_to(key, "otel_scope_name"), "{key}");
    }
    assert!(sanitizes_to("le", "le"));
    for key in ["le.", "1le", ".le", "lee"] {
        assert!(!sanitizes_to(key, "le"), "{key}");
    }
}

/// A histogram with explicit bucket `bounds` and one observation of `value`.
fn make_f64_histogram_with_bounds(bounds: Vec<f64>, value: f64) -> Histogram<f64> {
    let metrics = collect_metrics(|meter| {
        let histogram = meter
            .f64_histogram("bounded")
            .with_boundaries(bounds)
            .build();
        histogram.record(value, &[]);
    });
    metrics
        .scope_metrics()
        .flat_map(ScopeMetrics::metrics)
        .find_map(|metric| Histogram::<f64>::from_aggregated(metric.data()).cloned())
        .expect("the histogram was recorded")
}

#[test]
// om[verify histogram.negative-threshold-nosum]
fn test_write_histogram_omits_sum_with_negative_threshold() {
    let negative_threshold = make_f64_histogram_with_bounds(vec![-1.0, 1.0], 0.5);
    let mut output = String::new();
    write_histogram(
        &mut Context::with_output(&mut output).family("", "histogram", None),
        &negative_threshold,
    )
    .unwrap();
    assert!(!output.contains("_sum"), "{output}");
    assert!(!output.contains("_count"), "{output}");

    // A zero threshold is not negative.
    let zero_threshold = make_f64_histogram_with_bounds(vec![0.0, 1.0], 0.5);
    let mut output = String::new();
    write_histogram(
        &mut Context::with_output(&mut output).family("", "histogram", None),
        &zero_threshold,
    )
    .unwrap();
    assert!(output.contains("_sum"), "{output}");
}

#[test]
// om[verify exemplars.bucket-attachment]
// c[verify histogram.bucket.exemplar]
fn test_falls_in_bucket() {
    const BOUNDS: [f64; 3] = [1.0, 2.0, 3.0];

    for (bucket, value, expected) in [
        (0, 0.5, true),
        // A bound closes its own bucket and opens the next.
        (0, 1.0, true),
        (0, 1.5, false),
        (1, 1.0, false),
        (1, 1.5, true),
        (1, 2.0, true),
        (1, 5.0, false),
        // The +Inf bucket takes everything above the last finite bound.
        (3, 3.0, false),
        (3, 5.0, true),
    ] {
        assert_eq!(
            falls_in_bucket(&BOUNDS, bucket, &value),
            expected,
            "bucket {bucket}, value {value}"
        );
    }
}

#[test]
fn test_falls_in_bucket_value_types() {
    const BOUNDS: [f64; 3] = [1.0, 2.0, 3.0];

    // Values are widened to f64 to be compared against the bounds.
    assert!(falls_in_bucket(&BOUNDS, 3, &5u64));
    assert!(falls_in_bucket(&BOUNDS, 3, &5i64));
    assert!(falls_in_bucket(&BOUNDS, 0, &-5i64));

    // NaN belongs to no bucket at all.
    assert!(!falls_in_bucket(&BOUNDS, 0, &f64::NAN));
    assert!(!falls_in_bucket(&BOUNDS, 3, &f64::NAN));

    // Without finite bounds, +Inf is the only bucket there is.
    assert!(falls_in_bucket(&[], 0, &5.0));
}

#[test]
// om[verify metric.nointerleave] - all samples of one LabelSet (Metric) precede the next
// om[verify metricpoint.nointerleave] - count/sum/bucket samples of one point are contiguous
// om[verify histogram.inf-bucket]
// c[verify histogram.created]
// c[verify histogram.count]
// c[verify histogram.bucket.inf]
fn test_write_histogram() {
    let metric = make_f64_histogram_metric(vec![
        (125.0, vec![KeyValue::new("kk", "v1")]),
        (125.0, vec![KeyValue::new("kk", "v2")]),
        (25.0, vec![KeyValue::new("kk", "v1")]),
        (0.0, vec![KeyValue::new("kk", "v1")]),
        (25.0, vec![KeyValue::new("kk", "v2")]),
    ]);
    let ts = metric
        .time()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        .to_string();
    let start_ts = metric
        .start_time()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        .to_string();

    let mut output = String::new();

    let mut ctx = Context {
        attr_buffer: String::from("staledata"),
        scope_name: "myscope",
        ..Context::with_output(&mut output)
    };
    write_histogram(&mut ctx.family("myhistogram", "histogram", None), &metric).unwrap();
    let output = output.replace(&ts, "<TIMESTAMP>");
    let output = output.replace(&start_ts, "<START_TIMESTAMP>");

    assert_snapshot!(strip_otel_scope_name(&output));
}

#[test]
fn test_write_histogram_with_min_max() {
    let metric = make_f64_histogram_metric(vec![
        (125.0, vec![KeyValue::new("kk", "v1")]),
        (0.0, vec![KeyValue::new("kk", "v1")]),
        (25.0, vec![KeyValue::new("kk", "v2")]),
    ]);
    let ts = metric
        .time()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        .to_string();
    let start_ts = metric
        .start_time()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        .to_string();

    let mut output = String::new();
    let mut ctx = Context {
        attr_buffer: String::from("staledata"),
        scope_name: "myscope",
        config: Config::builder().histogram_min_max(true).build(),
        ..Context::with_output(&mut output)
    };
    write_histogram(&mut ctx.family("myhistogram", "histogram", None), &metric).unwrap();
    let output = output.replace(&ts, "<TIMESTAMP>");
    let output = output.replace(&start_ts, "<START_TIMESTAMP>");

    let output = strip_otel_scope_name(&output);
    assert!(output.contains("myhistogram_min{kk=\"v1\"} 0"));
    assert!(output.contains("myhistogram_max{kk=\"v1\"} 125"));
    assert!(output.contains("myhistogram_min{kk=\"v2\"} 25"));
    assert!(output.contains("myhistogram_max{kk=\"v2\"} 25"));
}

#[test]
// c[verify exphist.unimplemented]
fn test_drop_exponential_histogram() {
    let metric = make_f64_exponential_histogram_metric_handle(
        "my_exphist",
        None,
        Some("i didn't know this was possible"),
        vec![
            (125.0, vec![KeyValue::new("kk", "v1")]),
            (125.0, vec![KeyValue::new("kk", "v2")]),
            (25.0, vec![KeyValue::new("kk", "v1")]),
            (0.0, vec![KeyValue::new("kk", "v1")]),
            (25.0, vec![KeyValue::new("kk", "v2")]),
        ],
    );

    let mut output = String::new();

    assert!(extract_type_unit_and_name(&mut String::new(), &metric).is_none());

    metric
        .resource_metrics()
        .write_as_openmetrics(&mut output)
        .unwrap();
    assert!(!output.contains("my_exphist"));
}

#[test]
// c[verify sum.drop]
// c[verify histogram.delta]
fn test_drop_delta_temporality() {
    let metrics = collect_metrics_with_temporality(Temporality::Delta, |meter| {
        meter.u64_counter("my_counter").build().add(1, &[]);
        meter.i64_up_down_counter("my_updown").build().add(-1, &[]);
        meter.f64_histogram("my_histogram").build().record(1.0, &[]);
        meter.f64_gauge("my_gauge").build().record(1.0, &[]);
    });
    let collected: Vec<&str> = metrics
        .scope_metrics()
        .flat_map(ScopeMetrics::metrics)
        .map(Metric::name)
        .collect();
    assert_eq!(
        collected.len(),
        4,
        "the SDK dropped a metric: {collected:?}"
    );

    let mut output = String::new();
    metrics.write_as_openmetrics(&mut output).unwrap();

    for dropped in ["my_counter", "my_updown", "my_histogram"] {
        assert!(!output.contains(dropped), "{dropped} in:\n{output}");
    }
    assert!(output.contains("my_gauge"), "my_gauge missing:\n{output}");
}

#[test]
fn test_write_as_openmetrics() {
    let resource_metrics = make_test_metrics();
    let mut output = String::new();
    resource_metrics.write_as_openmetrics(&mut output).unwrap();

    // Verify the output has all expected structural elements
    // c[verify resource.target-info]
    assert!(output.starts_with("# TYPE target info\n"));
    assert!(output.contains("target_info{"));
    // c[verify scope.info]
    assert!(output.contains("# TYPE otel_scope info\n"));
    assert!(output.contains("otel_scope_info{"));
    // c[verify metadata.type]
    assert!(output.contains("# TYPE f64_gauge gauge\n"));
    // c[verify metadata.help-description]
    assert!(output.contains("# HELP f64_gauge "));
    assert!(output.contains("# TYPE u64_counter_seconds counter\n"));
    // om[verify metadata.unit-line]
    assert!(output.contains("# UNIT u64_counter_seconds seconds\n"));
    assert!(output.contains("u64_counter_seconds_total{"));
    assert!(output.contains("# TYPE histo histogram\n"));
    // c[verify histogram.created]
    assert!(output.contains("histo_created{"));
    // c[verify histogram.count]
    assert!(output.contains("histo_count{"));
    // c[verify histogram.sum]
    assert!(output.contains("histo_sum{"));
    // c[verify histogram.bucket]
    assert!(output.contains("histo_bucket{"));
    // om[verify text.eof]
    assert!(output.ends_with("# EOF\n"));
}

#[test]
// c[verify scope.config-disable] - disabling scope info in the config drops
// the info metrics and the scope labels
fn test_write_as_openmetrics_without_scope_info() {
    let resource_metrics = make_test_metrics();
    let mut output = String::new();
    resource_metrics
        .write_as_openmetrics_with_config(
            &mut output,
            Config::builder().scope_info_enabled(false).build(),
        )
        .unwrap();

    assert!(output.starts_with("# TYPE target info\n"));
    assert!(output.contains("target_info{"));
    assert!(!output.contains("# TYPE otel_scope info\n"));
    assert!(!output.contains("otel_scope_info{"));
    assert!(!output.contains("otel_scope_name="));
    assert!(!output.contains("otel_scope_version="));
}

#[test]
fn test_write_as_openmetrics_without_target_info() {
    let resource_metrics = make_test_metrics();
    let mut output = String::new();
    resource_metrics
        .write_as_openmetrics_with_config(
            &mut output,
            Config::builder().target_info_enabled(false).build(),
        )
        .unwrap();

    assert!(!output.contains("target_info"));
    assert!(output.contains("otel_scope_info{"));
    assert!(output.contains("otel_scope_name="));
    assert!(output.contains("otel_scope_version="));
}

#[test]
fn test_to_openmetrics_string_trait_method() {
    let resource_metrics = make_test_metrics();
    let result = resource_metrics.to_openmetrics_string().unwrap();

    assert!(result.contains("# EOF"));
    assert!(result.contains("# TYPE"));
}

#[test]
fn test_extract_type_unit_and_name_with_unit() {
    let handle = make_u64_counter_metric_handle(
        "u64.counter",
        Some("s"),
        None,
        vec![(125, vec![KeyValue::new("kk", "v1")])],
    );

    let mut name = String::from("stalename");
    let (typ, unit) = extract_type_unit_and_name(&mut name, &handle).unwrap();
    assert_eq!(typ, "counter");
    // om[verify metadata.unit-suffix]
    // c[verify metadata.unit-suffix]
    assert_eq!(name, "u64_counter_seconds");
    assert_eq!(unit, Some(std::borrow::Cow::Borrowed("seconds")));
}

#[test]
fn test_do_not_duplicate_unit() {
    let handle = make_u64_counter_metric_handle(
        "u64.per-second",
        Some("1/s"),
        None,
        vec![(125, vec![KeyValue::new("kk", "v1")])],
    );
    let mut name = String::new();
    let (_, unit) = extract_type_unit_and_name(&mut name, &handle).unwrap();
    // om[verify metadata.unit-suffix]
    // c[verify metadata.unit-suffix]
    assert_eq!(name, "u64_per_second");
    assert_eq!(unit, Some(std::borrow::Cow::Borrowed("per_second")));
}

#[test]
fn test_extract_type_unit_and_name_no_unit() {
    let resource_metrics = make_test_metrics();
    let scopes: Vec<&ScopeMetrics> = resource_metrics.scope_metrics().collect();

    // Find the histo metric which has no unit
    let hist_metric = scopes
        .iter()
        .flat_map(|s| s.metrics())
        .find(|m| m.name() == "histo")
        .expect("histo metric should exist");

    let mut name = String::new();
    let (typ, unit) = extract_type_unit_and_name(&mut name, hist_metric).unwrap();
    assert_eq!(typ, "histogram");
    assert_eq!(name, "histo");
    assert!(unit.is_none());
}

#[test]
// c[verify sum.total-suffix] - a pre-existing `_total` is stripped from the
// MetricFamily name only for counters; other types keep the suffix.
fn test_extract_strip_total_suffix() {
    let values = vec![(1, vec![KeyValue::new("k", "v")])];

    let counter = make_u64_counter_metric_handle("mycounter_total", None, None, values.clone());
    let mut name = String::new();
    assert!(extract_type_unit_and_name(&mut name, &counter).is_some());
    assert_eq!(name, "mycounter");

    let gauge = make_u64_gauge_metric_handle("gauge_total", None, None, values);
    assert!(extract_type_unit_and_name(&mut name, &gauge).is_some());
    assert_eq!(name, "gauge_total");
}

mod write_values {
    use super::*;

    fn strip_timestamps(mut text: String, mut timestamps: Vec<SystemTime>) -> String {
        timestamps.sort();
        timestamps.reverse();
        for ts in timestamps {
            let ts_string = ts
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs_f64()
                .to_string();
            text = text.replace(&ts_string, "<TIMESTAMP>");
        }
        text
    }

    #[test]
    // om[verify numbers.float]
    fn test_f64_sum() {
        let metric = make_f64_counter_metric(vec![(1.5, vec![KeyValue::new("k", "v")])]);
        let data = AggregatedMetrics::F64(MetricData::Sum(metric));

        let mut output = String::new();
        let mut ctx = Context {
            attr_buffer: String::from("staledata"),
            scope_name: "myscope",
            ..Context::with_output(&mut output)
        };
        write_values(&mut ctx.family("my_f64_sum", "counter", None), &data).unwrap();
        output = strip_timestamps(output, data.extract_timestamps());
        assert_snapshot!(strip_otel_scope_name(&output));
    }

    #[test]
    // om[verify numbers.integer]
    fn test_u64_gauge() {
        let metric = make_u64_gauge_metric(vec![(99, vec![KeyValue::new("k", "v")])]);
        let data = AggregatedMetrics::U64(MetricData::Gauge(metric));

        let mut output = String::new();
        let mut ctx = Context {
            attr_buffer: String::from("staledata"),
            scope_name: "myscope",
            ..Context::with_output(&mut output)
        };
        write_values(&mut ctx.family("my_u64_gauge", "gauge", None), &data).unwrap();
        output = strip_timestamps(output, data.extract_timestamps());
        assert_snapshot!(strip_otel_scope_name(&output));
    }

    #[test]
    // c[verify histogram.bucket]
    // c[verify histogram.bucket.le]
    // c[verify histogram.bucket.cumulative]
    // c[verify histogram.count]
    // c[verify histogram.sum]
    // c[verify histogram.created]
    fn test_u64_histogram() {
        let metric = make_u64_histogram_metric(vec![(50, vec![KeyValue::new("k", "v")])]);
        let data = AggregatedMetrics::U64(MetricData::Histogram(metric));

        let mut output = String::new();
        let mut ctx = Context {
            attr_buffer: String::from("staledata"),
            scope_name: "myscope",
            ..Context::with_output(&mut output)
        };
        write_values(&mut ctx.family("my_u64_hist", "histogram", None), &data).unwrap();
        output = strip_timestamps(output, data.extract_timestamps());
        assert_snapshot!(strip_otel_scope_name(&output));
    }

    #[test]
    // om[verify numbers.integer]
    fn test_i64_gauge() {
        let metric = make_i64_gauge_metric(vec![(-5, vec![KeyValue::new("k", "v")])]);
        let data = AggregatedMetrics::I64(MetricData::Gauge(metric));

        let mut output = String::new();
        let mut ctx = Context {
            attr_buffer: String::from("staledata"),
            scope_name: "myscope",
            ..Context::with_output(&mut output)
        };
        write_values(&mut ctx.family("my_i64_gauge", "gauge", None), &data).unwrap();
        output = strip_timestamps(output, data.extract_timestamps());
        assert_snapshot!(strip_otel_scope_name(&output));
    }

    #[test]
    // c[verify sum.cumulative-nonmonotonic.default] - i64_up_down_counter is non-monotonic
    fn test_i64_sum() {
        let metric = make_i64_counter_metric(vec![(42, vec![KeyValue::new("k", "v")])]);
        let data = AggregatedMetrics::I64(MetricData::Sum(metric));

        let mut output = String::new();
        let mut ctx = Context {
            attr_buffer: String::from("staledata"),
            scope_name: "myscope",
            ..Context::with_output(&mut output)
        };
        write_values(&mut ctx.family("my_i64_sum", "gauge", None), &data).unwrap();
        output = strip_timestamps(output, data.extract_timestamps());
        assert_snapshot!(strip_otel_scope_name(&output));
    }
}

/// The `# TYPE` lines of `text`, which carry no timestamps.
fn type_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| line.starts_with("# TYPE "))
        .collect()
}

#[test]
fn test_family_order_does_not_depend_on_creation_order() {
    // A gauge `b` and a counter `b` with unit `1` write the families `b` and `b_ratio`.
    let gauge_first = collect_metrics(|meter| {
        meter.u64_gauge("b").build().record(1, &[]);
        meter.u64_counter("b").with_unit("1").build().add(1, &[]);
    });
    let counter_first = collect_metrics(|meter| {
        meter.u64_counter("b").with_unit("1").build().add(1, &[]);
        meter.u64_gauge("b").build().record(1, &[]);
    });
    let gauge_first = gauge_first.to_openmetrics_string().unwrap();
    let counter_first = counter_first.to_openmetrics_string().unwrap();
    assert_eq!(type_lines(&gauge_first), type_lines(&counter_first));
}

#[test]
fn test_scope_order_does_not_depend_on_creation_order() {
    let scope = |version: &'static str| {
        InstrumentationScope::builder("s")
            .with_version(version)
            .build()
    };
    let record = |meters: &[opentelemetry::metrics::Meter]| {
        for meter in meters {
            meter.u64_gauge("g").build().record(1, &[]);
        }
    };
    let ascending = collect_from_scopes([scope("1"), scope("2")], record);
    let descending = collect_from_scopes([scope("2"), scope("1")], record);
    let info_lines = |metrics: &ResourceMetrics| {
        let text = metrics.to_openmetrics_string().unwrap();
        text.lines()
            .filter(|line| line.starts_with("otel_scope_info"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(info_lines(&ascending), info_lines(&descending));
}

fn scope(name: &'static str) -> InstrumentationScope {
    InstrumentationScope::builder(name).build()
}

#[test]
// c[verify metadata.no-duplicates]
// c[verify metadata.conflict-keep-points]
// om[verify metadata.unique]
fn test_one_metric_name_in_two_scopes_is_one_family() {
    let metrics = collect_from_scopes([scope("one"), scope("two")], |meters| {
        meters[0].u64_counter("requests").build().add(1, &[]);
        meters[1].u64_counter("requests").build().add(2, &[]);
    });
    let text = metrics.to_openmetrics_string().unwrap();
    assert_eq!(
        type_lines(&text),
        [
            "# TYPE target info",
            "# TYPE otel_scope info",
            "# TYPE requests counter"
        ],
        "{text}"
    );
    assert!(
        text.contains("requests_total{otel_scope_name=\"one\"} 1 "),
        "{text}"
    );
    assert!(
        text.contains("requests_total{otel_scope_name=\"two\"} 2 "),
        "{text}"
    );
}

#[test]
// c[verify metadata.no-duplicates]
fn test_one_metric_name_with_two_number_types_is_one_family() {
    let metrics = collect_metrics(|meter| {
        meter
            .u64_counter("a")
            .build()
            .add(1, &[KeyValue::new("k", "u")]);
        meter
            .f64_counter("a")
            .build()
            .add(1.5, &[KeyValue::new("k", "f")]);
    });
    let config = Config::builder()
        .scope_info_enabled(false)
        .target_info_enabled(false)
        .build();
    let text = metrics.to_openmetrics_string_with_config(config).unwrap();
    assert_eq!(type_lines(&text), ["# TYPE a counter"], "{text}");
    assert!(text.contains("a_total{k=\"f\"} 1.5 "), "{text}");
    assert!(text.contains("a_total{k=\"u\"} 1 "), "{text}");
}

#[test]
// c[verify metadata.conflict-type]
fn test_metrics_of_conflicting_types_keep_the_first_type() {
    let metrics = collect_metrics(|meter| {
        meter.u64_gauge("a").build().record(7, &[]);
        meter.u64_counter("a").build().add(1, &[]);
    });
    let config = Config::builder()
        .scope_info_enabled(false)
        .target_info_enabled(false)
        .build();
    let text = metrics.to_openmetrics_string_with_config(config).unwrap();
    // `counter` orders before `gauge`, so the counter is written and the gauge dropped.
    assert_eq!(type_lines(&text), ["# TYPE a counter"], "{text}");
    assert!(!text.contains(" 7 "), "{text}");
}

#[test]
// c[verify metadata.conflict-unit-help]
fn test_family_keeps_the_first_help() {
    let metrics = collect_from_scopes([scope("one"), scope("two")], |meters| {
        meters[0]
            .u64_counter("a")
            .with_description("first")
            .build()
            .add(1, &[]);
        meters[1]
            .u64_counter("a")
            .with_description("second")
            .build()
            .add(1, &[]);
    });
    let text = metrics.to_openmetrics_string().unwrap();
    let help: Vec<&str> = text.lines().filter(|l| l.starts_with("# HELP ")).collect();
    assert_eq!(help, ["# HELP a first"], "{text}");
}

#[test]
// om[verify metricfamily.name-clash]
fn test_family_with_a_clashing_sample_name_is_dropped() {
    let metrics = collect_metrics(|meter| {
        meter.u64_counter("a").build().add(1, &[]);
        meter.u64_gauge("a_total").build().record(7, &[]);
    });
    let config = Config::builder()
        .scope_info_enabled(false)
        .target_info_enabled(false)
        .build();
    let text = metrics.to_openmetrics_string_with_config(config).unwrap();
    // `a` orders before `a_total`, so the counter keeps the sample name `a_total`.
    assert_eq!(type_lines(&text), ["# TYPE a counter"], "{text}");
    assert!(!text.contains(" 7 "), "{text}");
}

#[test]
// om[verify metricfamily.name-clash]
fn test_family_clashing_with_target_info_is_dropped() {
    let metrics = collect_metrics(|meter| meter.u64_gauge("target_info").build().record(7, &[]));

    let with_target_info = Config::builder().scope_info_enabled(false).build();
    let text = metrics
        .to_openmetrics_string_with_config(with_target_info)
        .unwrap();
    assert_eq!(type_lines(&text), ["# TYPE target info"], "{text}");

    let without_target_info = Config::builder()
        .scope_info_enabled(false)
        .target_info_enabled(false)
        .build();
    let text = metrics
        .to_openmetrics_string_with_config(without_target_info)
        .unwrap();
    assert_eq!(type_lines(&text), ["# TYPE target_info gauge"], "{text}");
}

#[test]
fn test_families_without_a_shared_sample_name_are_kept() {
    // Histogram `a` writes `a_count`; counter `a_count` writes `a_count_total`.
    let metrics = collect_metrics(|meter| {
        meter.u64_histogram("a").build().record(1, &[]);
        meter.u64_counter("a_count").build().add(1, &[]);
    });
    let config = Config::builder()
        .scope_info_enabled(false)
        .target_info_enabled(false)
        .build();
    let text = metrics.to_openmetrics_string_with_config(config).unwrap();
    assert_eq!(
        type_lines(&text),
        ["# TYPE a histogram", "# TYPE a_count counter"],
        "{text}"
    );
}

#[test]
// om[verify metricfamily.unique-labelset]
fn test_points_repeating_a_label_set_are_dropped() {
    // Without scope labels, two scopes' points of one family are told apart by nothing.
    let metrics = collect_from_scopes([scope("one"), scope("two")], |meters| {
        meters[0].u64_gauge("g").build().record(1, &[]);
        meters[1].u64_gauge("g").build().record(2, &[]);
    });
    let config = Config::builder()
        .scope_info_enabled(false)
        .target_info_enabled(false)
        .build();
    let text = metrics.to_openmetrics_string_with_config(config).unwrap();
    let samples: Vec<&str> = text.lines().filter(|l| l.starts_with("g{")).collect();
    assert_eq!(samples.len(), 1, "{text}");
    assert!(samples[0].starts_with("g{} 1 "), "{text}");

    // An integer and a string attribute of one key both write `k="1"`.
    let metrics = collect_metrics(|meter| {
        let gauge = meter.u64_gauge("g").build();
        gauge.record(1, &[KeyValue::new("k", 1)]);
        gauge.record(2, &[KeyValue::new("k", "1")]);
    });
    let text = metrics.to_openmetrics_string_with_config(config).unwrap();
    assert_eq!(
        text.lines().filter(|l| l.starts_with("g{")).count(),
        1,
        "{text}"
    );
}

#[test]
fn test_kept_point_of_a_repeated_label_set_does_not_depend_on_recording_order() {
    let config = Config::builder()
        .scope_info_enabled(false)
        .target_info_enabled(false)
        .build();
    let sample = |integer_first: bool| {
        let metrics = collect_metrics(|meter| {
            let gauge = meter.u64_gauge("g").build();
            let integer = (1, KeyValue::new("k", 1));
            let string = (2, KeyValue::new("k", "1"));
            let (first, second) = if integer_first {
                (integer, string)
            } else {
                (string, integer)
            };
            gauge.record(first.0, &[first.1]);
            gauge.record(second.0, &[second.1]);
        });
        let text = metrics.to_openmetrics_string_with_config(config).unwrap();
        let sample = text.lines().find(|l| l.starts_with("g{")).unwrap();
        sample.split(' ').nth(1).unwrap().to_owned()
    };
    assert_eq!(sample(true), sample(false));
}

#[test]
fn test_series_are_written_in_label_order() {
    let metrics = collect_metrics(|meter| {
        let gauge = meter.u64_gauge("g").build();
        for value in ["h", "g", "f", "e", "d", "c", "b", "a"] {
            gauge.record(1, &[KeyValue::new("k", value)]);
        }
    });
    let config = Config::builder()
        .scope_info_enabled(false)
        .target_info_enabled(false)
        .build();
    let text = metrics.to_openmetrics_string_with_config(config).unwrap();
    let labels: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("g{"))
        .map(|l| &l[2..l.find('}').unwrap()])
        .collect();
    let mut sorted = labels.clone();
    sorted.sort_unstable();
    assert_eq!(labels.len(), 8, "{text}");
    assert_eq!(labels, sorted, "{text}");
}

#[test]
fn test_drop_repeated_label_sets_keeps_first_member_then_smallest_hash() {
    let text = r#"x="1"x="1"x="1"y="2""#;
    let point = |member, point, start| LabeledPoint {
        member,
        point,
        start,
        end: start + 5,
        key: 0,
    };
    let mut points = vec![
        point(1, 0, 0),
        point(0, 3, 5),
        point(0, 1, 10),
        point(0, 0, 15),
    ];
    drop_repeated_label_sets("f", text, &mut points, 2, |member| {
        assert_eq!(member, 0, "only member 0 holds a repeat of its own");
        vec![0, 9, 0, 5]
    });
    let kept: Vec<_> = points.iter().map(|p| (p.member, p.point)).collect();
    assert_eq!(kept, [(0, 3), (0, 0)]);
}

#[test]
fn test_sort_by_label_set_orders_as_bytes_do() {
    // Label sets from a small alphabet with NUL, around the 8-byte key boundaries.
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut random = |bound: u8| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        usize::try_from(seed % u64::from(bound)).expect("below a u8 bound")
    };
    for prefix in ["", "otel_scope_name=\"ottotom.bench\",system_device=\"dev"] {
        let mut text = String::new();
        let mut points = Vec::new();
        let mut push = |text: &mut String, member, label_set: &str| {
            let start = text.len();
            text.push_str(label_set);
            let point = points.len();
            points.push(LabeledPoint {
                member,
                point,
                start,
                end: text.len(),
                key: 0,
            });
        };
        push(&mut text, 0, "otel_metric_overflow=\"true\"");
        for _ in 0..500 {
            let len = random(20);
            let suffix: String = (0..len).map(|_| ['\0', 'a', 'b'][random(3)]).collect();
            push(&mut text, random(3), &format!("{prefix}{suffix}"));
        }
        // Label sets that share their bytes in steps of 8 reach the depth limit.
        for steps in 0..40 {
            push(&mut text, 0, &"abcdefgh".repeat(steps));
        }

        let mut expected = points.clone();
        expected.sort_by(|a, b| {
            text.as_bytes()[a.start..a.end]
                .cmp(&text.as_bytes()[b.start..b.end])
                .then(a.member.cmp(&b.member))
        });
        sort_by_label_set(text.as_bytes(), &mut points, 0, 0);
        let order = |points: &[LabeledPoint]| {
            let label_sets = points.iter().map(|p| (&text[p.start..p.end], p.member));
            label_sets.collect::<Vec<_>>()
        };
        assert_eq!(order(&points), order(&expected), "prefix {prefix:?}");
    }
}

#[test]
fn test_family_without_kept_points_writes_no_header() {
    // The gauge's only point shadows the reserved `otel_scope_name` label.
    let metrics = collect_metrics(|meter| {
        meter
            .u64_gauge("a")
            .with_unit("ms")
            .with_description("dropped")
            .build()
            .record(1, &[KeyValue::new("otel_scope_name", "x")]);
        meter.u64_counter("b").build().add(1, &[]);
    });
    let text = metrics.to_openmetrics_string().unwrap();
    assert_eq!(
        type_lines(&text),
        [
            "# TYPE target info",
            "# TYPE otel_scope info",
            "# TYPE b counter"
        ],
        "{text}"
    );
    assert!(!text.contains("milliseconds"), "{text}");
    assert!(!text.contains("dropped"), "{text}");
}

#[test]
fn test_series_order_does_not_depend_on_which_instrument_kind_records_first() {
    // An up-down counter is written as a gauge, so both instruments land in family `a`.
    let render = |gauge_first: bool| {
        let metrics = collect_metrics(|meter| {
            let gauge = || {
                meter
                    .f64_gauge("a")
                    .build()
                    .record(1.0, &[KeyValue::new("k", "g")]);
            };
            let sum = || meter.f64_up_down_counter("a").build().add(2.0, &[]);
            if gauge_first {
                gauge();
                sum();
            } else {
                sum();
                gauge();
            }
        });
        let config = Config::builder()
            .scope_info_enabled(false)
            .target_info_enabled(false)
            .build();
        let text = metrics.to_openmetrics_string_with_config(config).unwrap();
        text.lines()
            .filter(|l| l.starts_with("a{"))
            .map(|l| l.split(' ').take(2).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
    };
    assert_eq!(render(true), render(false));
}

#[test]
fn test_series_order_does_not_depend_on_which_unit_records_first() {
    // `a_seconds` without a unit and `a_seconds` in `s` share the family `a_seconds`.
    let render = |unitless_first: bool| {
        let metrics = collect_metrics(|meter| {
            let unitless = || meter.i64_up_down_counter("a_seconds").build().add(1, &[]);
            let seconds = || {
                meter
                    .i64_up_down_counter("a_seconds")
                    .with_unit("s")
                    .build()
                    .add(2, &[KeyValue::new("k", "s")]);
            };
            if unitless_first {
                unitless();
                seconds();
            } else {
                seconds();
                unitless();
            }
        });
        let config = Config::builder()
            .scope_info_enabled(false)
            .target_info_enabled(false)
            .build();
        let text = metrics.to_openmetrics_string_with_config(config).unwrap();
        text.lines()
            .filter(|l| l.starts_with("a_seconds{"))
            .map(|l| l.split(' ').take(2).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
    };
    assert_eq!(render(true), render(false));
}

#[test]
fn test_le_labels_follow_changing_bounds() {
    let mut le = LeLabels::default();
    let labels = |le: &LeLabels| {
        (0..le.bounds.len())
            .map(|i| le.label(i).to_owned())
            .collect::<Vec<_>>()
    };

    le.update([0.0, 1e6, 2.5].into_iter());
    assert_eq!(labels(&le), ["0.0", "1e+06", "2.5"]);
    le.update([0.0, 1e6, 2.5].into_iter());
    assert_eq!(labels(&le), ["0.0", "1e+06", "2.5"]);
    // -0.0 equals 0.0, but renders differently.
    le.update([-0.0, 1e6, 2.5].into_iter());
    assert_eq!(labels(&le), ["-0.0", "1e+06", "2.5"]);
    le.update([-0.0, 7.0].into_iter());
    assert_eq!(labels(&le), ["-0.0", "7.0"]);
    le.update([-0.0, 7.0, 1e-5, 8.0].into_iter());
    assert_eq!(labels(&le), ["-0.0", "7.0", "1e-05", "8.0"]);
    le.update(std::iter::empty());
    assert_eq!(labels(&le), [] as [String; 0]);
    assert_eq!(le.text, "");
}

#[test]
fn test_write_escaped_value_matches_as_str() {
    let strings = |values: &[&'static str]| {
        Value::Array(
            values
                .iter()
                .map(|&s| StringValue::from(s))
                .collect::<Vec<_>>()
                .into(),
        )
    };
    let values = [
        Value::Bool(true),
        Value::Bool(false),
        Value::I64(0),
        Value::I64(i64::MIN),
        Value::I64(i64::MAX),
        Value::F64(0.1),
        Value::F64(1.0),
        Value::F64(-0.0),
        Value::F64(1e21),
        Value::F64(5e-324),
        Value::F64(f64::NAN),
        Value::F64(f64::INFINITY),
        Value::F64(f64::NEG_INFINITY),
        Value::from("a\"b\\c\nd"),
        Value::Array(vec![true, false].into()),
        Value::Array(vec![-1i64, 2].into()),
        Value::Array(vec![1.5, f64::NAN].into()),
        Value::Array(Vec::<i64>::new().into()),
        strings(&["a\"b", "c\\d", "e\nf", ""]),
    ];
    for value in values {
        let mut expected = String::new();
        write_escaped(&mut expected, &value.as_str()).unwrap();
        let mut written = String::new();
        write_escaped_value(&mut written, &value).unwrap();
        assert_eq!(written, expected, "{value:?}");
    }
}
