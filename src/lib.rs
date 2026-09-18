#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

#[macro_use]
mod log;

/// Implementation of the OpenMetrics text format conversion.
pub mod convert;
/// Contains the main interface of this crate, [`exporter::OpenMetricsExporter`].
#[cfg(feature = "exporter")]
pub mod exporter;

mod format;

mod private {
    pub trait Sealed {}
}
