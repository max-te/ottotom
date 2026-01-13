use std::time::UNIX_EPOCH;

use insta::assert_snapshot;
use opentelemetry::KeyValue;
use opentelemetry_sdk::metrics::data::ScopeMetrics;
use ottotom_testsupport::metric_data::{
    make_f64_gauge_metric, make_f64_histogram_metric, make_u64_counter_metric,
};
use ottotom_testsupport::resource_metrics::make_test_metrics;
use ufmt::uwrite;

use super::*;

#[test]
fn test_write_sanitized_name() {
    let mut output = String::new();

    // Test with valid name
    write_sanitized_name(&mut output, "valid_metric_name").unwrap();
    assert_eq!(output, "valid_metric_name");

    // Test with name containing invalid characters
    output.clear();
    write_sanitized_name(&mut output, "invalid._ä.metric-name").unwrap();
    assert_eq!(output, "invalid_metric_name");

    // Test with name starting with digit
    output.clear();
    write_sanitized_name(&mut output, "1.metric").unwrap();
    assert_eq!(output, "_1_metric");
}

#[test]
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
fn test_make_scope_name_attrs() {
    let scope_name = "test_scope";
    let attr = make_scope_name_attrs(scope_name);

    if cfg!(feature = "otel_scope_info") {
        assert!(attr.is_some());
        if let Some(kv) = attr {
            assert_eq!(kv.key.as_str(), "otel_scope_name");
            assert_eq!(kv.value.as_str(), "test_scope");
        }
    } else {
        assert!(attr.is_none());
    }
}

#[test]
fn test_to_timestamp() {
    use std::time::{Duration, UNIX_EPOCH};

    // Test with a known timestamp
    let time = UNIX_EPOCH + Duration::from_secs(1625097600);
    let timestamp = to_timestamp(time);
    let mut output = String::new();
    uwrite!(output, "{}", timestamp).unwrap();
    assert_eq!(output, "1625097600");
}

#[cfg(feature = "otel_scope_info")]
#[test]
fn test_write_otel_scope_info() {
    let resource_metrics = make_test_metrics();
    let scopes: Vec<&ScopeMetrics> = resource_metrics.scope_metrics().collect();

    let mut output = String::new();
    write_otel_scope_info(&mut output, &scopes).unwrap();

    assert!(output.contains("# TYPE otel_scope info"));
    assert!(output.contains("otel_scope_info{"));
    assert!(output.contains("otel_scope_name=\"meter.1\""));
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
fn test_write_values() {
    let resource_metrics = make_test_metrics();
    let scopes: Vec<&ScopeMetrics> = resource_metrics.scope_metrics().collect();

    for scope in scopes {
        let scope_name = scope.scope().name();

        for metric in scope.metrics() {
            let mut output = String::new();
            let mut ctx = Context {
                scope_name,
                name: metric.name().to_owned(),
                attr_buffer: String::from("staledata"),
                ..Context::with_output(&mut output)
            };
            let result = write_values(&mut ctx, metric.data());

            assert!(result.is_ok());
            assert!(!output.is_empty());
            assert!(output.contains(metric.name()));
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
    assert_snapshot!(output);
}

#[test]
fn test_write_counter() {
    let metric = make_u64_counter_metric(vec![(125, vec![KeyValue::new("kk", "v1")])]);
    let ts = metric
        .time()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        .to_string();

    let mut output = String::new();

    let mut ctx = Context {
        attr_buffer: String::from("staledata"),
        name: "mycounter".to_owned(),
        scope_name: "myscope",
        ..Context::with_output(&mut output)
    };
    write_counter(&mut ctx, &metric).unwrap();

    let output = output.replace(&ts, "<TIMESTAMP>");
    assert_snapshot!(output);
}

#[test]
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

    assert_snapshot!(output);
}
