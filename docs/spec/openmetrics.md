# OpenMetrics Text Format

Specification: <https://github.com/prometheus/OpenMetrics/blob/main/specification/OpenMetrics.md>

## Overall Structure

om[text.utf8]
UTF-8 MUST be used. Byte order markers MUST NOT be used.

om[text.contenttype]
The content type MUST be `application/openmetrics-text; version=1.0.0; charset=utf-8`.

om[text.lineending]
Line endings MUST be signalled with LF (`\n`) and MUST NOT contain CR (`\r`).

om[text.eof]
Expositions MUST end with the `# EOF` line and SHOULD end with `EOF\n`.

## Escaping

om[escaping.chars]
Escaping MUST be applied: LF → `\n`, `"` → `\"`, `\` → `\\`.

## Metric Metadata

om[metadata.order]
The ordering of metadata lines SHOULD be TYPE, UNIT, HELP.

om[metadata.type-unknown]
If no TYPE is exposed, the MetricFamily MUST be of type Unknown.

om[metadta.type-values]
The TYPE value MUST be one of "unknown", "gauge", "counter", "stateset", "info", "histogram", "gaugehistogram", and "summary".

om[metadata.unit-line]
If a unit is specified it MUST be provided in a UNIT metadata line.

om[metadata.unit-suffix]
An underscore and the unit MUST be the suffix of the MetricFamily name.

om[metadata.unique]
There MUST NOT be more than one of each type of metadata line per MetricFamily.

om[metadata.hash-lines]
Lines beginning with `#` aside from metadata and EOF MUST NOT be exposed.

## Metric Types

### Histogram

om[histogram.buckets]
A Histogram MetricPoint MUST contain at least one bucket.

om[histogram.inf-bucket]
Histogram MetricPoints MUST have one bucket with an `+Inf` threshold.

om[histogram.sorted]
Buckets MUST be sorted in increasing order of `le`, and `le` values MUST follow Canonical Numbers.

om[histogram.nole]
A Histogram's Metric's LabelSet MUST NOT have a `le` label name.

om[histogram.nonanneg]
Sum and bucket values MUST NOT be NaN or negative.

om[histogram.threshold-nonan]
Bucket thresholds MUST NOT equal NaN.

om[histogram.integers]
Count and bucket values MUST be integers.

### Counter

om[counter.suffix]
The MetricPoint's Total Value Sample MetricName MUST have the suffix `_total`. If present the MetricPoint's Created Value Sample MetricName MUST have the suffix `_created`.

### StateSet

om[stateset.suffix]
The Sample MetricName for a StateSet MetricPoint MUST NOT have a suffix.

om[stateset.one-sample-per-state]
StateSets MUST have one sample per State in the MetricPoint.

om[stateset.label-name]
Each sample MUST have a label with the MetricFamily name as the label name and the State name as the label value. A StateSet Metric's LabelSet MUST NOT have a label name which is the same as the name of its MetricFamily.

om[stateset.boolean-values]
Each sample value MUST be 1 if the State is true and MUST be 0 if the State is false.

om[stateset.enum]
If encoded as an ENUM, StateSets MUST have exactly one boolean which is true within a MetricPoint.

om[stateset.unit-empty]
MetricFamilies of type StateSet MUST have an empty Unit string.

### Unknown

om[unknown.suffix]
The Sample MetricName for the value of a MetricPoint for a MetricFamily of type Unknown MUST NOT have a suffix.

om[unknown.single]
A MetricPoint in a metric with the Unknown type MUST have a single value.

### Info

om[info.suffix]
The Sample MetricName for an Info MetricPoint MUST have the suffix `_info`.

om[info.value]
The Sample value of an Info MetricPoint MUST always be 1.

om[info.unit-empty]
MetricFamilies of type Info MUST have an empty Unit string.

## Labels

om[labels.reserved]
Label names beginning with underscores are RESERVED and MUST NOT be used unless specified by this standard.

om[labels.unique]
Label names MUST be unique within a LabelSet.

## Exemplars

om[exemplars.structure]
Exemplars MUST consist of a LabelSet and a value, and MAY have a timestamp. They MAY each be different from the MetricPoints' LabelSet and timestamp.

om[exemplars.length-limit]
The combined length of the label names and values of an Exemplar's LabelSet MUST NOT exceed 128 UTF-8 character code points.

om[exemplars.bucket-attachment]
Bucket values MAY have exemplars. Each bucket covers the values less than or equal to it, and the value of the exemplar MUST be within this range. Exemplars SHOULD be put into the bucket with the highest value. A bucket MUST NOT have more than one exemplar.

om[exemplars.empty-labelset]
Exemplars without Labels MUST represent an empty LabelSet as `{}` in the text format.


## Numbers

om[numbers.integer]
Integer numbers MUST NOT have a decimal point.

om[numbers.float]
Floating point numbers MUST be represented with a decimal point or scientific notation.

om[numbers.canonical-inf]
Exposers MUST produce output for positive infinity as `+Inf`.

## Timestamps

om[timestamp.unix]
Timestamps MUST be Unix Epoch in seconds.

om[timestamp.monotonic]
If more than one MetricPoint is exposed for a Metric, then its MetricPoints MUST have monotonically increasing timestamps.

## MetricFamily and MetricSet

### Data Model

The exposition is organized as a hierarchy: a MetricSet contains MetricFamilies,
each MetricFamily contains Metrics, and each Metric contains MetricPoints.

A **MetricPoint** consists of a set of values, depending on the MetricFamily
type. For example, a histogram MetricPoint has Count, Sum, and Bucket values,
while a counter MetricPoint has a single Total value. MetricPoints of the same
Metric are distinguished by their timestamps.

A **Metric** is defined by a unique LabelSet within a MetricFamily and MUST
contain a list of one or more MetricPoints. Metrics with the same name for a
given MetricFamily SHOULD have the same set of label names in their LabelSet.

A **MetricFamily** is a group of Metrics sharing a name, HELP, TYPE, and UNIT
metadata. Every Metric within a MetricFamily MUST have a unique LabelSet.

A **MetricSet** is the top level object exposed by OpenMetrics. It MUST consist
of MetricFamilies and MAY be empty.

### Ordering

om[metricfamily.nointerleave]
MetricFamilies MUST NOT be interleaved.

om[metric.nointerleave]
Metrics MUST NOT be interleaved.

om[metricpoint.nointerleave]
MetricPoints MUST NOT be interleaved.

om[metricfamily.name-clash]
The name of a MetricFamily MUST NOT result in a potential clash for sample metric names with another MetricFamily.

## Strings

om[strings.utf8]
Strings MUST only consist of valid UTF-8 characters. NULL (ASCII 0x0) MUST be supported.
