use std::ops::DerefMut;
use std::sync::Arc;
use std::time::Duration;

use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use tokio::sync::{Mutex, RwLock};

use crate::convert::WriteOpenMetrics;

/// A [PushMetricExporter] which writes metrics into an internal buffer in OpenMetrics text format.
#[derive(Debug, Clone)]
pub struct OpenMetricsExporter {
    buffer: Arc<RwLock<String>>,
    backbuffer: Arc<Mutex<String>>,
}

impl Default for OpenMetricsExporter {
    fn default() -> Self {
        OpenMetricsExporter {
            buffer: Arc::new(RwLock::new(String::new())),
            backbuffer: Arc::new(Mutex::new(String::new())),
        }
    }
}

impl OpenMetricsExporter {
    #[deprecated(note = "use Default::default() instead")]
    pub fn new() -> Self {
        Default::default()
    }

    /// Get a clone of the last-exported OpenMetrics text.
    pub async fn text(&self) -> String {
        self.buffer.read().await.as_str().to_owned()
    }
}

impl PushMetricExporter for OpenMetricsExporter {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        #[cfg(feature = "tracing")]
        tracing::debug!("Exporting metrics");
        let mut backbuffer = self.backbuffer.lock().await;
        backbuffer.clear();
        metrics
            .write_as_openmetrics(backbuffer.deref_mut())
            .map_err(|err| {
                OTelSdkError::InternalFailure(format!("Failed to write to buffer: {err}"))
            })?;

        let mut frontbuffer = self.buffer.write().await;
        std::mem::swap(frontbuffer.deref_mut(), backbuffer.deref_mut());

        Ok(())
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        Ok(())
    }

    fn temporality(&self) -> Temporality {
        Temporality::Cumulative
    }
}
