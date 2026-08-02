# OTLP Metric Points to Prometheus

Specification: <https://github.com/open-telemetry/opentelemetry-specification/blob/v1.45.0/specification/compatibility/prometheus_and_openmetrics.md>

## Metric Metadata

c[metadata.no-duplicates]
Prometheus Pull exporters MUST NOT allow duplicate UNIT, HELP, or TYPE comments for the same metric name in a single scrape.

c[metadata.conflict-type]
Exporters MUST drop entire metrics to prevent conflicting TYPE comments.

c[metadata.conflict-unit-help]
All but one of conflicting UNIT and HELP comments (but not metric points) SHOULD be dropped.

c[metadata.drop-warn]
If dropping a comment or metric points, the exporter SHOULD warn the user through error logging.

c[metadata.name-sanitize]
The Name of an OTLP metric MUST be added as the
[Prometheus Metric Name](https://prometheus.io/docs/instrumenting/exposition_formats/#comments-help-text-and-type-information),
with unit and type suffixes added as described below. The metric name is
required to match the regex: `[a-zA-Z_:]([a-zA-Z0-9_:])*`. Invalid characters
in the metric name MUST be replaced with the `_` character. Multiple
consecutive `_` characters MUST be replaced with a single `_` character.

c[metadata.unit-convert]
The Unit of an OTLP metric point SHOULD be converted to the equivalent unit in Prometheus when possible.  This includes:

 * Converting from abbreviations to full words (e.g. "ms" to "milliseconds").
 * Dropping the portions of the Unit within brackets (e.g. {packet}). Brackets MUST NOT be included in the resulting unit. A "count of foo" is considered unitless in Prometheus.
 * Special case: Converting "1" to "ratio".
 * Converting "foo/bar" to "foo_per_bar".

c[metadata.unit-suffix]
The resulting unit SHOULD be added to the metric as
[UNIT metadata](https://github.com/prometheus/OpenMetrics/blob/v1.0.0/specification/OpenMetrics.md#metricfamily)
and as a suffix to the metric name unless the metric name already ends with the
unit (before type-specific suffixes), or the unit metadata MUST be omitted. The
unit suffix comes before any type-specific suffixes.

c[metadata.help-description]
The description of an OTLP metrics point MUST be added as
[HELP metadata](https://prometheus.io/docs/instrumenting/exposition_formats/#comments-help-text-and-type-information).

c[metadata.type]
The data point type of an OTLP metric MUST be added as
[TYPE metadata](https://prometheus.io/docs/instrumenting/exposition_formats/#comments-help-text-and-type-information).
It also dictates type-specific conversion rules listed below.

## Instrumentation Scope

c[scope.info]
Prometheus exporters SHOULD generate an [Info](https://github.com/prometheus/OpenMetrics/blob/v1.0.0/specification/OpenMetrics.md#info)-typed
metric named `otel_scope_info` for each Instrumentation Scope with non-empty
scope attributes.

c[scope.name-version]
If present, Instrumentation Scope `name` and `version` MUST
be added as `otel_scope_name` and `otel_scope_version` labels on the `otel_scope_info` metric.

c[scope.attribute-labels]
Scope attributes
MUST also be added as labels following the rules described in the
[`Metric Attributes`](#metric-attributes) section below.

c[scope.labels-on-points]
Prometheus exporters MUST add the scope name as the `otel_scope_name` label and
the scope version as the `otel_scope_version` label on all metric points by
default, based on the scope the original data point was nested in.

c[scope.config-disable]
Prometheus exporters SHOULD provide a configuration option to disable the
`otel_scope_info` metric and `otel_scope_` labels.


## Gauges

(Requires metric metadata)
An OpenTelemetry Gauge MUST be converted to a Prometheus Unknown-typed metric if the `prometheus.type` key of metric metadata is `unknown`. Otherwise, it MUST be converted to a Prometheus Gauge.

## Sums

c[sum.cumulative-monotonic]
A cumulative, monotonic Sum MUST be converted to a Prometheus Counter.

(Requires metric metadata)
A cumulative, non-monotonic Sum with `prometheus.type=info` MUST be converted to an OpenMetrics Info metric.

(Requires metric metadata)
A cumulative, non-monotonic Sum with `prometheus.type=stateset` MUST be converted to an OpenMetrics StateSet metric.

c[sum.cumulative-nonmonotonic.default]
A cumulative, non-monotonic Sum MUST be converted to a Prometheus Gauge when no more specific conversion rule applies.

c[sum.delta-monotonic]
A delta, monotonic Sum SHOULD be converted to cumulative temporality and become a Prometheus Counter.
The new data point type must be the same as the accumulated data point type.
The new data point's start time must match the time of the accumulated data point.

c[sum.drop]
Sums not matching any conversion rule MUST be dropped.

c[sum.total-suffix]
If the metric name for a monotonic Sum does not end in `_total`, a `_total` suffix MUST be added by default, otherwise the name MUST remain unchanged. Exporters SHOULD provide a configuration option to disable the addition of `_total` suffixes.

c[sum.created]
Monotonic Sum metric points with `StartTimeUnixNano` SHOULD export the `{name}_created` metric.

## Histograms

c[histogram.count]
A cumulative Histogram MUST be converted to a Prometheus metric family.

c[histogram.count]
A cumulative Histogram MUST be converted to a Prometheus metric family with
a single `{name}_count` metric denoting the count field of the histogram. All attributes of the histogram point are converted to Prometheus labels.

c[histogram.sum]
A cumulative Histogram MUST be converted to a Prometheus metric family with a
`{name}_sum` metric denoting the sum field of the histogram, reported only if the sum is positive and monotonic. The sum is positive and monotonic when all buckets are positive. All attributes of the histogram point are converted to Prometheus labels.

c[histogram.bucket]
A cumulative Histogram MUST be converted to a Prometheus metric family with
a series of `{name}_bucket` metric points that contain all attributes of the histogram point recorded as labels.

c[histogram.bucket.le]
`_bucket` points MUST include a `le` label. The label's value is the stringified floating point value of bucket boundaries, ordered from lowest to highest.

c[histogram.bucket.cumulative]
The value of each point is the sum of the count of all histogram buckets up to the boundary reported in the `le` label.

c[histogram.bucket.exemplar]
These points will include a single exemplar that falls within `le` label and no other `le` labelled point.

c[histogram.bucket.inf]
The final bucket MUST have a `+Inf` threshold.

c[histogram.created]
Histograms with `StartTimeUnixNano` set SHOULD export the `{name}_created` metric.

c[histogram.delta]
OpenTelemetry Histograms with Delta aggregation temporality SHOULD be aggregated into a Cumulative aggregation temporality and follow the logic above, or MUST be dropped.


## Exponential Histograms

> c[exphist.unimplemented]
> Exponential histograms MUST be dropped as input
> until they are implemented.

An OpenTelemetry Exponential Histogram with
a cumulative aggregation temporality MUST be converted to a Prometheus Native
Histogram as follows:

- `Scale` is converted to the Native Histogram `Schema`. Currently,
  [valid values](https://github.com/prometheus/prometheus/commit/d9d51c565c622cdc7d626d3e7569652bc28abe15#diff-bdaf80ebc5fa26365f45db53435b960ce623ea6f86747fb8870ad1abc355f64fR76-R83)
  for `schema` are -4 <= n <= 8.
  If `Scale` is > 8 then Exponential Histogram data points SHOULD be downscaled
  to a scale accepted by Prometheus (in range [-4,8]). Any data point unable to
  be rescaled to an acceptable range MUST be dropped.
- `Count` is converted to Native Histogram `Count` if the `NoRecordedValue`
  flag is set to `false`, otherwise, Native Histogram `Count` is set to the
  Stale NaN value.
- `Sum` is converted to the Native Histogram `Sum` if `Sum` is set and the
  `NoRecordedValue` flag is set to `false`, otherwise, Native Histogram `Sum` is
  set to the Stale NaN value.
- `TimeUnixNano` is converted to the Native Histogram `Timestamp` after
  converting nanoseconds to milliseconds.
- `ZeroCount` is converted directly to the Native Histogram `ZeroCount`.
- `ZeroThreshold`, if set, is converted to the Native Histogram `ZeroThreshold`.
  Otherwise, it is set to the default value `1e-128`.
- The dense bucket layout represented by `Positive` bucket counts and `Offset` is
  converted to the Native Histogram sparse layout represented by `PositiveSpans`
  and `PositiveDeltas`. The same holds for the `Negative` bucket counts
  and `Offset`. Note that Prometheus Native Histograms buckets are indexed by
  upper boundary while Exponential Histograms are indexed by lower boundary, the
  result being that the Offset fields are different-by-one.
- `Min` and `Max` are not used.
- `StartTimeUnixNano` is not used.

OpenTelemetry Exponential Histogram]
metrics with the delta aggregation temporality are dropped.

## Summaries

> Summaries don't exist in the `opentelemetry-rust` SDK data model,
> only at the protocol layer for Prometheus summaries.
> Thus, these rules are not applicable.

An OpenTelemetry Summary MUST be converted to a Prometheus metric family with
a single `{name}_count` metric denoting the count field of the summary.
All attributes of the summary point are converted to Prometheus labels.

An OpenTelemetry Summary MUST be converted to a Prometheus metric family with a
`{name}_sum` metric denoting the sum field of the summary, reported
only if the sum is positive and monotonic. All attributes of the summary
point are converted to Prometheus labels.

An OpenTelemetry Summary MUST be converted to a Prometheus metric family with 
A series of `{name}` metric points that contain all attributes of the
summary point recorded as labels.  Additionally, a label, denoted as
`quantile` is added denoting a reported quantile point, and having its value
be the stringified floating point value of quantiles (between 0.0 and 1.0),
starting from lowest to highest, and all being non-negative.  The value of
each point is the computed value of the quantile point.

Summaries with `StartTimeUnixNano` set SHOULD export the `{name}_created` metric.


## Metric Attributes

c[mattrs.to-labels]
OpenTelemetry Metric Attributes MUST be converted to
[Prometheus labels](https://prometheus.io/docs/concepts/data_model/#metric-names-and-labels).

c[mattrs.type-conversion]
String Attribute values are converted directly to Metric Attributes, and
non-string Attribute values MUST be converted to string attributes following
the attribute specification.

c[mattrs.key-sanitize]
Prometheus
metric label keys are required to match the following regex:
`[a-zA-Z_]([a-zA-Z0-9_])*`.  Metrics from OpenTelemetry with unsupported
Attribute names MUST replace invalid characters with the `_` character.
Multiple consecutive `_` characters MUST be replaced with a single `_`
character.

c[mattrs.key-sanitize.collisions]
If multiple key-value pairs are converted to have the same Prometheus
key, the values MUST be concatenated together, separated by `;`, and ordered by
the lexicographical order of the original keys.

## Exemplars

Exemplars on OpenTelemetry Histograms and Monotonic Sums SHOULD
be converted to Prometheus exemplars. Exemplars on other OpenTelemetry data
points MUST be dropped. For Prometheus Remote Write exporters, multiple exemplars are
able to be added to each bucket, so all exemplars SHOULD be converted. For
Prometheus pull endpoints, only a single exemplar is able to be added to each
bucket, so the largest exemplar from each bucket MUST be used, if attaching
exemplars. If no exemplars exist on a bucket, the highest exemplar from a lower
bucket MUST be used, even though it is a duplicate of another bucket's exemplar.
Prometheus Exemplars MUST use the `trace_id` and `span_id` keys for the trace
and span IDs, respectively. Timestamps MUST be added as timestamps on the
Prometheus exemplar, and `filtered_attributes` MUST be added as labels on the
Prometheus exemplar unless they would exceed the
[limit on characters](https://github.com/prometheus/OpenMetrics/blob/v1.0.0/specification/OpenMetrics.md#exemplars).

## Resource Attributes

c[resource.target-info]
In Prometheus exporters, an OpenTelemetry Resource SHOULD be converted to
a [`target` info metric](https://github.com/prometheus/OpenMetrics/blob/v1.0.0/specification/OpenMetrics.md#supporting-target-metadata-in-both-push-based-and-pull-based-systems)
if the resource is not empty.

c[resource.attrs]
The Resource attributes MAY be copied to labels of exported metric families
if required by the exporter configuration, or MUST be dropped.

c[resource.target-labels]
The `target`
info metric MUST be an info-typed
metric whose labels MUST include the resource attributes, and MUST NOT include
any other labels.

c[resource.attrs-key-sanitize]
To convert OTLP resource attributes to Prometheus labels, string Attribute values are converted directly to labels, and non-string Attribute values MUST be converted to string attributes following the attribute specification.

# Attributes

Specification: <https://github.com/open-telemetry/opentelemetry-specification/blob/v1.45.0/specification/common/README.md>

c[attrs.stringify]
For protocols that do not natively support non-string values, non-string values SHOULD be represented as JSON-encoded strings. For example, the expression int64(100) will be encoded as 100, float64(1.5) will be encoded as 1.5, and an empty array of any type will be encoded as [].
