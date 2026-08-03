use std::time::{Duration, SystemTime, UNIX_EPOCH};

use insta::assert_snapshot;
use opentelemetry::{Array, KeyValue, Value};
use opentelemetry_sdk::metrics::data::{MetricData, ScopeMetrics};
use ottotom_testsupport::metric_data::{
    make_f64_counter_metric, make_f64_exponential_histogram_metric_handle, make_f64_gauge_metric,
    make_f64_histogram_metric, make_i64_counter_metric, make_i64_gauge_metric,
    make_u64_counter_metric_handle, make_u64_gauge_metric, make_u64_gauge_metric_handle,
    make_u64_histogram_metric,
};
use ottotom_testsupport::resource_metrics::make_test_metrics;
use ottotom_testsupport::timestamps::ExtractTimestamps;
use ufmt::uwrite;

use super::*;

fn strip_otel_scope_name(s: &str) -> String {
    let mut result = s.to_owned();

    const OTEL_SCOPE_NAME: &str = "otel_scope_name=\"myscope\"";
    while let Some(start) = result.find(OTEL_SCOPE_NAME) {
        result.replace_range(start..start + OTEL_SCOPE_NAME.len(), "");
        if result.as_bytes()[start - 1..start] == *b"," {
            // Had preceding attributes
            result.replace_range(start - 1..start, "");
        } else if result.as_bytes()[start..start + 1] == *b"," {
            // Was first attribute with trailing attributes
            result.replace_range(start..start + 1, "");
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
    let attrs = make_scope_name_attrs(&Config::default(), scope_name, None);
    assert_eq!(attrs.len(), 1);
    assert_eq!(attrs[0].key.as_str(), "otel_scope_name");
    assert_eq!(attrs[0].value.as_str(), "test_scope");

    let scope_version = "1.2.3";
    let attrs = make_scope_name_attrs(&Config::default(), scope_name, Some(scope_version));
    assert_eq!(attrs.len(), 2);
    assert_eq!(attrs[0].key.as_str(), "otel_scope_name");
    assert_eq!(attrs[0].value.as_str(), "test_scope");
    assert_eq!(attrs[1].key.as_str(), "otel_scope_version");
    assert_eq!(attrs[1].value.as_str(), "1.2.3");

    let disabled = Config::builder().scope_info_enabled(false).build();
    let attrs = make_scope_name_attrs(&disabled, scope_name, None);
    assert!(attrs.is_empty());
    let attrs = make_scope_name_attrs(&disabled, scope_name, Some(scope_version));
    assert!(attrs.is_empty());
}

#[test]
// om[verify timestamp.unix]
fn test_to_timestamp() {
    use std::time::{Duration, UNIX_EPOCH};

    // Test with a known timestamp
    let time = UNIX_EPOCH + Duration::from_secs(1625097600);
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
    write_otel_scope_info(&mut output, &scopes, &Config::default()).unwrap();

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
                "Unexpected metric type: {}",
                type_str
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
        name: "mygauge".to_owned(),
        scope_name: "myscope",
        ..Context::with_output(&mut output)
    };

    write_gauge(&mut ctx, &metric).unwrap();
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
        assert!(extract_type_unit_and_name(&mut ctx, &handle));
        write_counter(&mut ctx, &sum).unwrap();

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

fn exemplar_from_parts<T>(
    value: T,
    time: SystemTime,
    filtered_attributes: Vec<KeyValue>,
    span_id: [u8; 8],
    trace_id: [u8; 16],
) -> Exemplar<T> {
    /// Mirror of [`Exemplar`]'s fields for constructing exemplars in tests.
    ///
    /// The fields are never read: the struct only exists to lay the bytes out
    /// identically to `Exemplar` before they are reinterpreted.
    #[expect(dead_code)]
    struct RawExemplar<T> {
        filtered_attributes: Vec<KeyValue>,
        time: SystemTime,
        value: T,
        span_id: [u8; 8],
        trace_id: [u8; 16],
    }

    // `exemplar_from_parts` reinterprets `RawExemplar`'s bytes as an `Exemplar`, so
    // the layouts must match. `Exemplar`'s fields are `pub(crate)`, so size and
    // alignment are the strongest cross-crate checks available; these fail to
    // compile if the SDK's `Exemplar` changes size or alignment.
    const _: () =
        assert!(std::mem::size_of::<RawExemplar<f64>>() == std::mem::size_of::<Exemplar<f64>>());
    const _: () =
        assert!(std::mem::align_of::<RawExemplar<f64>>() == std::mem::align_of::<Exemplar<f64>>());
    const _: () =
        assert!(std::mem::size_of::<RawExemplar<u64>>() == std::mem::size_of::<Exemplar<u64>>());
    const _: () =
        assert!(std::mem::align_of::<RawExemplar<u64>>() == std::mem::align_of::<Exemplar<u64>>());
    const _: () =
        assert!(std::mem::size_of::<RawExemplar<i64>>() == std::mem::size_of::<Exemplar<i64>>());
    const _: () =
        assert!(std::mem::align_of::<RawExemplar<i64>>() == std::mem::align_of::<Exemplar<i64>>());

    // SAFETY: `RawExemplar` mirrors `Exemplar`'s fields, types, and order (see
    // `opentelemetry_sdk::metrics::data::Exemplar`), so the layouts are
    // identical and reading the mirror's bytes as an `Exemplar` is sound.
    // `mem::forget` keeps the mirror from dropping the `Vec` now owned by the
    // returned `Exemplar`.
    //
    // `Exemplar` has no public constructor, so this builds a mirror struct with
    // the same fields, types, and order as `Exemplar<T>` and reinterprets its
    // bytes. The compile-time size/alignment assertions above and the
    // `test_exemplar_roundtrip` test below verify the layouts stay in sync.
    unsafe {
        let raw = RawExemplar {
            filtered_attributes,
            time,
            value,
            span_id,
            trace_id,
        };
        let exemplar = std::ptr::read(&raw as *const RawExemplar<T> as *const Exemplar<T>);
        std::mem::forget(raw);
        exemplar
    }
}

#[test]
// om[verify exemplars.structure]
// om[verify exemplars.empty-labelset]
// c[verify exemplar.trace-span-ids]
// c[verify exemplar.filtered-attrs]
// c[verify exemplar.timestamp]
fn test_write_exemplar() {
    let exemplar = exemplar_from_parts(
        0.67,
        UNIX_EPOCH + Duration::from_millis(123_456),
        vec![KeyValue::new("filtered", "yes")],
        [0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0xba, 0xbe],
        [
            0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab,
            0xcd, 0xef,
        ],
    );
    let mut output = String::new();
    write_exemplar(&mut output, std::iter::once(&exemplar)).unwrap();
    assert_eq!(
        output,
        " # {filtered=\"yes\",span_id=\"deadbeefcafebabe\",trace_id=\"1234567890abcdef1234567890abcdef\"} 0.67 123.456"
    );

    // Zeroed ids and no filtered attributes render as an empty label set.
    let bare = exemplar_from_parts::<f64>(1.0, UNIX_EPOCH, vec![], [0; 8], [0; 16]);
    let mut output = String::new();
    write_exemplar(&mut output, std::iter::once(&bare)).unwrap();
    assert_eq!(output, " # {} 1.0 0.0");
}

#[test]
// Verifies every field survives the byte reinterpretation in
// `exemplar_from_parts` unchanged, catching field reordering or resizing in
// `opentelemetry_sdk`'s `Exemplar` that the size/alignment assertions above
// cannot.
fn test_exemplar_roundtrip() {
    let filtered = vec![KeyValue::new("filtered", "yes")];
    // Obvious test data: span_id `0xdeadbeefcafebabe`, trace_id
    // `0x1234567890abcdef1234567890abcdef`.
    let span_id = [0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0xba, 0xbe];
    let trace_id = [
        0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd,
        0xef,
    ];
    let time = UNIX_EPOCH + Duration::from_millis(123_456);
    let exemplar = exemplar_from_parts(0.67, time, filtered.clone(), span_id, trace_id);

    assert_eq!(exemplar.value, 0.67);
    assert_eq!(exemplar.time(), time);
    assert_eq!(*exemplar.trace_id(), trace_id);
    assert_eq!(*exemplar.span_id(), span_id);
    assert_eq!(
        exemplar.filtered_attributes().collect::<Vec<_>>(),
        filtered.iter().collect::<Vec<_>>()
    );
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
        name: "myhistogram".to_owned(),
        scope_name: "myscope",
        ..Context::with_output(&mut output)
    };
    write_histogram(&mut ctx, &metric).unwrap();
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
        name: "myhistogram".to_owned(),
        scope_name: "myscope",
        config: Config::builder().histogram_min_max(true).build(),
        ..Context::with_output(&mut output)
    };
    write_histogram(&mut ctx, &metric).unwrap();
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

    let mut ctx = Context::with_output(&mut output);
    assert!(!extract_type_unit_and_name(&mut ctx, &metric));

    metric
        .resource_metrics()
        .write_as_openmetrics(&mut output)
        .unwrap();
    assert!(!output.contains("my_exphist"));
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

    let mut output = String::new();
    let mut ctx = Context {
        attr_buffer: String::from("staledata"),
        ..Context::with_output(&mut output)
    };

    let result = extract_type_unit_and_name(&mut ctx, &handle);
    assert!(result);
    assert_eq!(ctx.typ, "counter");
    // om[verify metadata.unit-suffix]
    // c[verify metadata.unit-suffix]
    assert_eq!(ctx.name, "u64_counter_seconds");
    assert_eq!(ctx.unit, Some(std::borrow::Cow::Borrowed("seconds")));
}

#[test]
fn test_do_not_duplicate_unit() {
    let handle = make_u64_counter_metric_handle(
        "u64.per-second",
        Some("1/s"),
        None,
        vec![(125, vec![KeyValue::new("kk", "v1")])],
    );
    let mut output = String::new();
    let mut ctx = Context {
        attr_buffer: String::from("staledata"),
        ..Context::with_output(&mut output)
    };
    let result = extract_type_unit_and_name(&mut ctx, &handle);
    assert!(result);
    // om[verify metadata.unit-suffix]
    // c[verify metadata.unit-suffix]
    assert_eq!(ctx.name, "u64_per_second");
    assert_eq!(ctx.unit, Some(std::borrow::Cow::Borrowed("per_second")));
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

    let mut output = String::new();
    let mut ctx = Context {
        attr_buffer: String::from("staledata"),
        ..Context::with_output(&mut output)
    };

    let result = extract_type_unit_and_name(&mut ctx, hist_metric);
    assert!(result);
    assert_eq!(ctx.typ, "histogram");
    assert_eq!(ctx.name, "histo");
    assert!(ctx.unit.is_none());
}

#[test]
// c[verify sum.total-suffix] - a pre-existing `_total` is stripped from the
// MetricFamily name only for counters; other types keep the suffix.
fn test_extract_strip_total_suffix() {
    let values = vec![(1, vec![KeyValue::new("k", "v")])];

    let counter = make_u64_counter_metric_handle("mycounter_total", None, None, values.clone());
    let mut counter_output = String::new();
    let mut ctx = Context::with_output(&mut counter_output);
    assert!(extract_type_unit_and_name(&mut ctx, &counter));
    assert_eq!(ctx.name, "mycounter");

    let gauge = make_u64_gauge_metric_handle("gauge_total", None, None, values);
    let mut gauge_output = String::new();
    let mut ctx = Context::with_output(&mut gauge_output);
    assert!(extract_type_unit_and_name(&mut ctx, &gauge));
    assert_eq!(ctx.name, "gauge_total");
}

mod write_values {
    use super::*;

    fn strip_timestamps(mut text: String, timestamps: Vec<SystemTime>) -> String {
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
            name: "my_f64_sum".to_owned(),
            scope_name: "myscope",
            ..Context::with_output(&mut output)
        };
        write_values(&mut ctx, &data).unwrap();
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
            name: "my_u64_gauge".to_owned(),
            scope_name: "myscope",
            ..Context::with_output(&mut output)
        };
        write_values(&mut ctx, &data).unwrap();
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
            name: "my_u64_hist".to_owned(),
            scope_name: "myscope",
            ..Context::with_output(&mut output)
        };
        write_values(&mut ctx, &data).unwrap();
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
            name: "my_i64_gauge".to_owned(),
            scope_name: "myscope",
            ..Context::with_output(&mut output)
        };
        write_values(&mut ctx, &data).unwrap();
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
            name: "my_i64_sum".to_owned(),
            scope_name: "myscope",
            ..Context::with_output(&mut output)
        };
        write_values(&mut ctx, &data).unwrap();
        output = strip_timestamps(output, data.extract_timestamps());
        assert_snapshot!(strip_otel_scope_name(&output));
    }
}
