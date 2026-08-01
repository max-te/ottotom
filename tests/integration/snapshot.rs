use std::time::SystemTime;

use insta::assert_snapshot;
use opentelemetry::Key;
use ottotom::convert::WriteOpenMetrics;

use ottotom_testsupport::resource_metrics::make_test_metrics;
use ottotom_testsupport::timestamps::get_all_timestamps;

#[test]
// om[verify metadata.order] - snapshot locks TYPE/UNIT/HELP ordering
// om[verify info.value] - info samples carry value 1
// om[verify metricfamily.nointerleave]
// c[verify resource.target-labels]
fn matches_snapshot() {
    let metrics = make_test_metrics();
    let erasable_timestamps = get_all_timestamps(&metrics);
    let mut formatted = metrics.to_openmetrics_string().unwrap();

    // Mask SDK version to avoid snapshot breakage when dependency resolution
    // yields a different patch version.
    let sdk_version_key = Key::from_static_str("telemetry.sdk.version");
    if let Some(sdk_version) = metrics.resource().get(&sdk_version_key) {
        formatted = formatted.replace(&*sdk_version.as_str(), "<SDK_VERSION>");
    }

    for (i, ts) in erasable_timestamps.iter().enumerate().rev() {
        let ts = ts
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            .to_string();
        formatted = formatted.replace(&ts, &format!("<TIMESTAMP_{}>", i));
    }

    assert_snapshot!(formatted);
}
