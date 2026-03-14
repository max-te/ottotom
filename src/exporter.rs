use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;

use crate::convert::WriteOpenMetrics;

/// A [`PushMetricExporter`] which writes metrics into an internal buffer in OpenMetrics text format.
#[derive(Debug, Clone)]
pub struct OpenMetricsExporter {
    buffer: Arc<RwLock<String>>,
    backbuffer: Arc<Mutex<String>>,
}

impl Default for OpenMetricsExporter {
    fn default() -> Self {
        Self {
            buffer: Arc::new(RwLock::new(String::new())),
            backbuffer: Arc::new(Mutex::new(String::new())),
        }
    }
}

impl OpenMetricsExporter {
    #[deprecated(note = "use Default::default() instead")]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a clone of the last-exported OpenMetrics text.
    pub async fn text(&self) -> String {
        match self.buffer.read().map(|t| t.as_str().to_owned()) {
            Ok(text) => text,
            Err(_) => std::future::pending().await,
            // TODO: turn this into a non-async function in 0.32
        }
    }
}

impl PushMetricExporter for OpenMetricsExporter {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        #[cfg(feature = "tracing")]
        tracing::debug!("Exporting metrics");
        let mut backbuffer = self.backbuffer.lock().map_err(|err| {
            OTelSdkError::InternalFailure(format!("Failed to acquire backbuffer: {err}"))
        })?;
        backbuffer.clear();
        metrics
            .write_as_openmetrics(&mut *backbuffer)
            .map_err(|err| {
                OTelSdkError::InternalFailure(format!("Failed to write to buffer: {err}"))
            })?;

        let mut frontbuffer = self.buffer.write().map_err(|err| {
            OTelSdkError::InternalFailure(format!(
                "Failed to acquire frontbuffer for writer: {err}"
            ))
        })?;
        std::mem::swap(&mut *frontbuffer, &mut *backbuffer);

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
