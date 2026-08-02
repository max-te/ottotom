use std::time::{SystemTime, UNIX_EPOCH};

use insta::assert_snapshot;
use opentelemetry::KeyValue;
use opentelemetry_sdk::metrics::data::{MetricData, ScopeMetrics};
use ottotom_testsupport::metric_data::{
    make_f64_counter_metric, make_f64_exponential_histogram_metric_handle, make_f64_gauge_metric,
    make_i64_counter_metric, make_i64_gauge_metric, make_u64_counter_metric,
    make_u64_counter_metric_handle, make_u64_gauge_metric,
};
#[cfg(not(feature = "experimental-histogram-min-max"))]
use ottotom_testsupport::metric_data::{make_f64_histogram_metric, make_u64_histogram_metric};
use ottotom_testsupport::resource_metrics::make_test_metrics;
use ottotom_testsupport::timestamps::ExtractTimestamps;
use ufmt::uwrite;

use super::*;

fn strip_otel_scope_name(s: &str) -> String {
    let mut result = s.to_owned();
    if !cfg!(feature = "otel_scope_info") {
        return result;
    }

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

    // Multiple consecutive invalid chars should collapse to single underscore
    output.clear();
    write_sanitized_name(&mut output, "a..b").unwrap();
    assert_eq!(output, "a_b");

    // Mixed invalid chars should collapse
    output.clear();
    write_sanitized_name(&mut output, "a..-..b").unwrap();
    assert_eq!(output, "a_b");

    // Leading invalid char becomes underscore
    output.clear();
    write_sanitized_name(&mut output, ".abc").unwrap();
    assert_eq!(output, "_abc");

    // Colons are allowed
    output.clear();
    write_sanitized_name(&mut output, "my:metric:name").unwrap();
    assert_eq!(output, "my:metric:name");

    // Colons mixed with invalid chars
    output.clear();
    write_sanitized_name(&mut output, "my:metric.name").unwrap();
    assert_eq!(output, "my:metric_name");
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
// c[verify scope.config-disable] - behavior differs with/without the feature
// c[verify scope.name-version] - scope name and version become info attributes
fn test_make_scope_name_attrs() {
    let scope_name = "test_scope";
    let attrs = make_scope_name_attrs(scope_name, None);

    if cfg!(feature = "otel_scope_info") {
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].key.as_str(), "otel_scope_name");
        assert_eq!(attrs[0].value.as_str(), "test_scope");
    } else {
        assert!(attrs.is_empty());
    }

    let scope_version = "1.2.3";
    let attrs = make_scope_name_attrs(scope_name, Some(scope_version));

    if cfg!(feature = "otel_scope_info") {
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].key.as_str(), "otel_scope_name");
        assert_eq!(attrs[0].value.as_str(), "test_scope");
        assert_eq!(attrs[1].key.as_str(), "otel_scope_version");
        assert_eq!(attrs[1].value.as_str(), "1.2.3");
    } else {
        assert!(attrs.is_empty());
    }
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

#[cfg(feature = "otel_scope_info")]
#[test]
// c[verify scope.info]
// c[verify scope.name-version]
fn test_write_otel_scope_info() {
    let resource_metrics = make_test_metrics();
    let scopes: Vec<&ScopeMetrics> = resource_metrics.scope_metrics().collect();

    let mut output = String::new();
    write_otel_scope_info(&mut output, &scopes).unwrap();

    assert!(output.contains("# TYPE otel_scope info"));
    assert!(output.contains("otel_scope_info{"));
    assert!(output.contains("otel_scope_name=\"meter.1\""));
    assert!(output.contains("otel_scope_version="));
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
    assert_snapshot!(strip_otel_scope_name(&output));

    let mut output2 = String::new();
    let mut ctx = Context {
        name: "mycounter_total".to_owned(),
        scope_name: "myscope",
        ..Context::with_output(&mut output2)
    };
    write_counter(&mut ctx, &metric).unwrap();
    let output2 = output2.replace(&ts, "<TIMESTAMP>");

    assert_eq!(output, output2);
}

#[cfg(not(feature = "experimental-histogram-min-max"))]
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
    assert!(output.starts_with("# TYPE target info\n") == cfg!(feature = "otel_scope_info"));
    assert!(output.contains("target_info{") == cfg!(feature = "otel_scope_info"));
    // c[verify scope.info]
    assert!(output.contains("# TYPE otel_scope info\n") == cfg!(feature = "otel_scope_info"));
    assert!(output.contains("otel_scope_info{") == cfg!(feature = "otel_scope_info"));
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

    #[cfg(not(feature = "experimental-histogram-min-max"))]
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
    // c[verify gauge-default]
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
