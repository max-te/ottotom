use opentelemetry::metrics::MeterProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use ottotom::exporter::OpenMetricsExporter;

#[test]
fn exporter_exports() {
    let exporter = OpenMetricsExporter::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter.clone())
        .build();
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let initial_text = rt.block_on(exporter.text());
    assert_eq!(initial_text, String::new());

    let meter = meter_provider.meter("meter.one");
    let gauge = meter.f64_gauge("a_gauge").build();
    gauge.record(42.0, &[]);

    meter_provider.force_flush().unwrap();
    let metrics_text = rt.block_on(exporter.text());
    assert!(metrics_text.contains("# TYPE a_gauge"));
}
