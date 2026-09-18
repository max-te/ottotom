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
    /// The most recent complete rendering. Replaced wholesale by [`Self::export`],
    /// so readers hand out a shared handle instead of copying the text.
    buffer: Arc<RwLock<Arc<str>>>,
    /// Scratch space for the in-progress rendering, kept between exports to reuse its capacity.
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
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            buffer: Arc::new(RwLock::new(Arc::from(""))),
            backbuffer: Arc::new(Mutex::new(String::new())),
            config,
        }
    }

    /// Get a handle to the last-exported OpenMetrics text.
    ///
    /// The text is not copied: the returned [`Arc`] shares the rendering that
    /// [`PushMetricExporter::export`] produced, and a later export leaves it
    /// untouched. An exporter that has not exported yet yields an empty string.
    #[must_use]
    pub fn text(&self) -> Arc<str> {
        self.buffer.read().map_or_else(
            |err| {
                error!("Frontbuffer lock was poisoned: {err}");
                // The frontbuffer only ever holds a finished rendering — it is
                // replaced by a single store — so a poisoned lock cannot expose
                // half-written text.
                Arc::clone(&err.into_inner())
            },
            |t| Arc::clone(&t),
        )
    }
}

impl PushMetricExporter for OpenMetricsExporter {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        debug!("Exporting metrics");
        let mut backbuffer = self.backbuffer.lock().unwrap_or_else(|err| {
            error!("Backbuffer lock was poisoned: {err}");
            self.backbuffer.clear_poison();
            err.into_inner()
        });
        backbuffer.clear();
        metrics
            .write_as_openmetrics_with_config(&mut *backbuffer, self.config)
            .map_err(|err| {
                OTelSdkError::InternalFailure(format!("Failed to write to buffer: {err}"))
            })?;
        let rendered: Arc<str> = Arc::from(backbuffer.as_str());
        drop(backbuffer);

        let mut frontbuffer = self.buffer.write().unwrap_or_else(|err| {
            error!("Frontbuffer lock was poisoned: {err}");
            self.buffer.clear_poison();
            err.into_inner()
        });
        *frontbuffer = rendered;

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
