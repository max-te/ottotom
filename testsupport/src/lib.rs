//! Test fixtures for the `ottotom` test suite: real `opentelemetry-sdk` metric
//! data, built through a `ManualReader`.
//
// Consumed only by `ottotom`'s own tests and benches, where every constructor's
// result is used immediately and a panic is the intended failure mode, so two
// of the pedantic API-documentation lints are off here.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc)]

pub mod metric_data;
pub mod reader;
pub mod resource_metrics;
pub mod timestamps;
