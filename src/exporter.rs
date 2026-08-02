use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;

use crate::convert::{Config, WriteOpenMetrics};

/// A [`PushMetricExporter`] which writes metrics into an internal buffer in OpenMetrics text format.
#[derive(Debug, Clone)]
pub struct OpenMetricsExporter {
    buffer: Arc<RwLock<String>>,
    backbuffer: Arc<Mutex<String>>,
    config: Config,
}

impl Default for OpenMetricsExporter {
    fn default() -> Self {
        Self::new(Config::default())
    }
}

impl OpenMetricsExporter {
    /// Create a new exporter with the given conversion [`Config`].
    pub fn new(config: Config) -> Self {
        Self {
            buffer: Arc::new(RwLock::new(String::new())),
            backbuffer: Arc::new(Mutex::new(String::new())),
            config,
        }
    }

    /// Get a clone of the last-exported OpenMetrics text.
    pub fn text(&self) -> String {
        self.buffer.read().map_or_else(
            |err| {
                tracing::error!("Frontbuffer lock was poisoned: {err}");
                // the frontbuffer-backbuffer swap should make sure we never see a corrupted buffer
                err.into_inner().as_str().to_owned()
            },
            |t| t.as_str().to_owned(),
        )
    }
}

impl PushMetricExporter for OpenMetricsExporter {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        #[cfg(feature = "tracing")]
        tracing::debug!("Exporting metrics");
        let mut backbuffer = self.backbuffer.lock().unwrap_or_else(|err| {
            tracing::error!("Backbuffer lock was poisoned: {err}");
            self.backbuffer.clear_poison();
            err.into_inner()
        });
        backbuffer.clear();
        metrics
            .write_as_openmetrics_with_config(&mut *backbuffer, self.config)
            .map_err(|err| {
                OTelSdkError::InternalFailure(format!("Failed to write to buffer: {err}"))
            })?;

        let mut frontbuffer = self.buffer.write().unwrap_or_else(|err| {
            tracing::error!("Frontbuffer lock was poisoned: {err}");
            self.buffer.clear_poison();
            err.into_inner()
        });
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
