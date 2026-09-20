use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arc_swap::ArcSwap;
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
    buffer: Arc<ArcSwap<String>>,
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
            buffer: Arc::new(ArcSwap::from_pointee(String::new())),
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
    pub fn text(&self) -> Arc<String> {
        self.buffer.load_full()
    }

    /// Render `metrics` into the backbuffer and publish it to the frontbuffer.
    fn render(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
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
        let rendered = backbuffer.clone();
        drop(backbuffer);

        self.buffer.store(Arc::new(rendered));

        Ok(())
    }
}

impl PushMetricExporter for OpenMetricsExporter {
    fn export(&self, metrics: &ResourceMetrics) -> impl Future<Output = OTelSdkResult> + Send {
        ready(self.render(metrics))
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        Ok(())
    }

    // `Cumulative` is also the SDK's default; the mutant is equivalent.
    #[cfg_attr(test, mutants::skip)]
    fn temporality(&self) -> Temporality {
        Temporality::Cumulative
    }
}
