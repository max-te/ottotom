#![doc = include_str!("../README.md")]

/// Implementation of the OpenMetrics text format conversion.
pub mod convert;
/// Contains the main interface of this crate, [`exporter::OpenMetricsExporter`].
#[cfg(feature = "exporter")]
pub mod exporter;

mod format;
