## [0.33.0-alpha.2] - 2026-08-20

### Features

- [`d6a3ad9`](https://github.com/max-te/ottotom/commit/d6a3ad9267b80a82e9a4f1732a4f6febe065e3d7) *(config)* Split settings for scope/target info

### Bug Fixes

- [`78b1cfd`](https://github.com/max-te/ottotom/commit/78b1cfd6baab6cdea36dccecb5efe168d551e7d5) Publish whole workspace in publish task
- [`72e8289`](https://github.com/max-te/ottotom/commit/72e82892022fb7a7ce8496fbee0e0c6ba6381e6a) *(ci)* Skip doctests on --no-default-features

### Other

- [`826a607`](https://github.com/max-te/ottotom/commit/826a607334624e343d50483a6372caa54d5942e2) Merge pull request #1 from max-te/dependabot/github_actions/actions/checkout-7

chore(deps): bump actions/checkout from 6 to 7
## [0.33.0-alpha.1] - 2026-08-03

### Features

- [`372d7c2`](https://github.com/max-te/ottotom/commit/372d7c23bf75fbaa10c5406f3fcaff480e11ebcf) *(convert)* Emit `_created` sample for monotonic sums
- [`e1a7e38`](https://github.com/max-te/ottotom/commit/e1a7e38ff0cfa77a10b8b7cef90b2f665f574aa6) *(convert)* Write exemplars on histograms and monotonic sums

### Bug Fixes

- [`aa6ceaa`](https://github.com/max-te/ottotom/commit/aa6ceaa04c8c1576cd2423d9b2fcde0e7b688b37) Clippy lints
- [`37bc8fe`](https://github.com/max-te/ottotom/commit/37bc8fe0c58d0d2d49dbb199d70d998886ae173e) *(clippy)* Apply lint suggestions
- [`1f57610`](https://github.com/max-te/ottotom/commit/1f57610914e797a10c138cd58206d75c427d7051) *(convert)* Drop delta-temporality sums
- [`ad9e633`](https://github.com/max-te/ottotom/commit/ad9e633248dc2bf29f5e95dadd2685138f23ab26) *(convert)* Handle case where unit is already part of name
- [`06862a0`](https://github.com/max-te/ottotom/commit/06862a05f09c33882237fed2a0974072b5a0d761) *(format)* Floats MUST have a decimal point or scientific notation
- [`57f3bfb`](https://github.com/max-te/ottotom/commit/57f3bfb98ee0eaee6d500a9f873e0494b44d653f) *(convert)* Only emit histogram _sum for non-negative values
- [`1244a78`](https://github.com/max-te/ottotom/commit/1244a78cffbdf8174349be424836b633aee9f9cc) *(convert)* Skip _total suffix when name already ends in _total
- [`6e804b5`](https://github.com/max-te/ottotom/commit/6e804b5b24d97d20c8d9eb25d93931ed81ca9cdd) *(convert)* Sanitize attribute keys for Prometheus labels
- [`fb2ff27`](https://github.com/max-te/ottotom/commit/fb2ff2731d1f592a9b4cb7c5074141ebafe5217a) *(convert)* Emit histogram _created once per label set

### Documentation

- [`cbeab4a`](https://github.com/max-te/ottotom/commit/cbeab4af01a421142fe467a2b20ba5b296149756) *(tracey)* Document openmetrics metrics/metricspoint/metricsfamily distinction
- [`b1da36f`](https://github.com/max-te/ottotom/commit/b1da36f278713bce7fd7c1eaa216bb245ef358d2) *(readme)* Add Tracey and spec-tracking section
- [`a58063c`](https://github.com/max-te/ottotom/commit/a58063c2533b6dd1cfe54cf0427d17ada2d94983) *(readme)* Update specs note
## [0.32.0] - 2026-05-10

### Documentation

- [`93eb3d0`](https://github.com/max-te/ottotom/commit/93eb3d0aa99b43fdd775de10fc2e374e16265ef1) Remove async from readme example

### Other

- [`0f5fcc3`](https://github.com/max-te/ottotom/commit/0f5fcc33de82682f10038c3c4cf1c0885b0aed5b) Clean up some pedantic clippy lints
- [`cbcc22f`](https://github.com/max-te/ottotom/commit/cbcc22fc0104c69ce7a463b700e6b9ffdbf0a462) Set up Github Actions test workflow
- [`8c80aa9`](https://github.com/max-te/ottotom/commit/8c80aa91b0cf66b064b335ceed87bfdb5b98d908) This crate doesn't need asynchronous locks
- [`8987f7b`](https://github.com/max-te/ottotom/commit/8987f7bbabbcdf4cf4089334ad530cb1a4f91bd8) Update to 0.32
## [0.31.3] - 2026-01-13

### Features

- [`561e89a`](https://github.com/max-te/ottotom/commit/561e89a6910fd9e5555dd24af073c852455aae49) Implement metrics exposition format myself
- [`5b00df6`](https://github.com/max-te/ottotom/commit/5b00df6c80a5f034d122399eea926569b625bda7) *(openmetrics)* Implement otel_scope_info
- [`383ca83`](https://github.com/max-te/ottotom/commit/383ca83d7604d8f2dbd6236c2d978cfe96a8a645) Unit conversion from opentelemetry-prometheus
- [`3962e41`](https://github.com/max-te/ottotom/commit/3962e418eb4205d7b7108bea33fa7e93c59e47db) Expose target_info metric

### Bug Fixes

- [`621f68d`](https://github.com/max-te/ottotom/commit/621f68d75fbef779e78c7ec570f335176726f66d) Escape was missing escaped char
- [`ffd82d6`](https://github.com/max-te/ottotom/commit/ffd82d6346f78ce8297290e2a8fcd82e4d00b7f1) Missing space
- [`cfda8a5`](https://github.com/max-te/ottotom/commit/cfda8a58cf28cb6372ade95395135702c42b83c0) Missing eof marker
- [`707230e`](https://github.com/max-te/ottotom/commit/707230e666cddbefd1b3bd1552b4efa24bd1acb8) Fix openmetrics correctness errors and implement parser test
- [`9403c63`](https://github.com/max-te/ottotom/commit/9403c6386575dacdc19089a73b3d84ab03d353fb) *(exporter)* Handle unsupported ExponentialHistogram metrics safely
- [`0f3040e`](https://github.com/max-te/ottotom/commit/0f3040ed4a5ec31856be56e9e98f044d1c4b7d74) Ensure deterministic output through sorting
- [`038e584`](https://github.com/max-te/ottotom/commit/038e5847bf8d60ea372c64974ae35b207ae107cb) *(write_sanitized_name)* Collapse underscores in metric names
- [`c31964e`](https://github.com/max-te/ottotom/commit/c31964ee0b452b7ddf12e6c2e840d556bcdd6a10) Histogram-min-max feature compiles again

### Performance

- [`2b11535`](https://github.com/max-te/ottotom/commit/2b115351291bfba6eefec45349b2ce0315bc8c5e) *(exporter)* Reduce allocations in OpenMetrics formatter by reusing temporary buffer
- [`11b4cb0`](https://github.com/max-te/ottotom/commit/11b4cb0591d823e5c970769d212e5851cf625aad) Use ufmt internally, 47% speedup

### Documentation

- [`eb473e1`](https://github.com/max-te/ottotom/commit/eb473e186e327b6106db037af1aad75b8e2445a8) Throw together a README
- [`30aab6d`](https://github.com/max-te/ottotom/commit/30aab6dc7462ac37bd7944ef1b251d5e6533a7a7) Add docsctring
- [`e82dc20`](https://github.com/max-te/ottotom/commit/e82dc207264e0510c53086930935706ed3c76261) Slim down README

### Other

- [`6746b42`](https://github.com/max-te/ottotom/commit/6746b42a293e31ad6a2ddafcbb41c799d3152f04) Use itoa & ryu for openmetrics formatting
- [`c1f7887`](https://github.com/max-te/ottotom/commit/c1f7887246277b491e2e2dac5b8f59692b6a9c06) Reserve enough space for escaping
- [`f7f9d5c`](https://github.com/max-te/ottotom/commit/f7f9d5ce3ca4bc0b98da5ce9c15c5920742cf590) Add rustdocs
- [`3bc416f`](https://github.com/max-te/ottotom/commit/3bc416f1d51362cceef5588688e19dbb71ece873) Add crate descriptions
- [`667ae6d`](https://github.com/max-te/ottotom/commit/667ae6dcc9cffed2222abfc1ea78e0dd84f68a6e) Update repository urls
- [`104020c`](https://github.com/max-te/ottotom/commit/104020cf510a2a3d3d4813324759e4b396059fc5) Exclude lockfile, this is a library
