use std::time::SystemTime;

use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ResourceMetrics};

pub trait ExtractTimestamps {
    fn extract_timestamps(&self) -> Vec<SystemTime>;
}

impl<T> ExtractTimestamps for MetricData<T> {
    fn extract_timestamps(&self) -> Vec<SystemTime> {
        let mut timestamps = Vec::new();
        match self {
            MetricData::Gauge(gauge) => {
                timestamps.push(gauge.time());
                gauge.start_time().inspect(|&time| timestamps.push(time));
            }
            MetricData::Sum(sum) => {
                timestamps.push(sum.time());
                timestamps.push(sum.start_time());
            }
            MetricData::Histogram(histogram) => {
                timestamps.push(histogram.time());
                timestamps.push(histogram.start_time());
            }
            MetricData::ExponentialHistogram(exponential_histogram) => {
                timestamps.push(exponential_histogram.time());
                timestamps.push(exponential_histogram.start_time());
            }
        }
        timestamps.sort_unstable();
        timestamps.dedup();
        timestamps
    }
}

impl ExtractTimestamps for AggregatedMetrics {
    fn extract_timestamps(&self) -> Vec<SystemTime> {
        match self {
            AggregatedMetrics::F64(metric_data) => metric_data.extract_timestamps(),
            AggregatedMetrics::U64(metric_data) => metric_data.extract_timestamps(),
            AggregatedMetrics::I64(metric_data) => metric_data.extract_timestamps(),
        }
    }
}

impl ExtractTimestamps for ResourceMetrics {
    fn extract_timestamps(&self) -> Vec<SystemTime> {
        let mut timestamps = Vec::new();
        for scope in self.scope_metrics() {
            for metric in scope.metrics() {
                timestamps.extend(metric.data().extract_timestamps());
            }
        }
        timestamps.sort_unstable();
        timestamps.dedup();
        timestamps
    }
}

pub fn get_all_timestamps(metrics: &ResourceMetrics) -> Vec<SystemTime> {
    metrics.extract_timestamps()
}
