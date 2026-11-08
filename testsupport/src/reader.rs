use std::sync::{Arc, Weak};

use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::reader::MetricReader;
use opentelemetry_sdk::metrics::{InstrumentKind, ManualReader, Pipeline, Temporality};

#[derive(Debug, Clone)]
pub struct TestMetricsReader {
    inner: Arc<ManualReader>,
    temporality: Temporality,
}

impl TestMetricsReader {
    /// Creates a reader that requests `temporality` for every instrument kind.
    pub fn with_temporality(temporality: Temporality) -> Self {
        Self {
            inner: Arc::new(ManualReader::builder().build()),
            temporality,
        }
    }
}

impl Default for TestMetricsReader {
    fn default() -> Self {
        Self::with_temporality(Temporality::Cumulative)
    }
}

impl MetricReader for TestMetricsReader {
    fn register_pipeline(&self, pipeline: Weak<Pipeline>) {
        self.inner.register_pipeline(pipeline);
    }

    fn collect(&self, rm: &mut ResourceMetrics) -> OTelSdkResult {
        self.inner.collect(rm)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown(&self) -> OTelSdkResult {
        self.inner.shutdown()
    }

    fn shutdown_with_timeout(&self, timeout: std::time::Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn temporality(&self, _kind: InstrumentKind) -> Temporality {
        self.temporality
    }
}
