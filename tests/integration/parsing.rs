use openmetrics_parser::{ParseError, openmetrics::parse_openmetrics};
use ottotom::convert::WriteOpenMetrics;

use ottotom_testsupport::resource_metrics::make_test_metrics;

#[test]
pub fn test_output_is_parseable_by_openmetrics_parser() {
    let metrics = make_test_metrics();

    let formatted = metrics.to_openmetrics_string().unwrap();
    println!("{}", &formatted);

    let parsed = parse_openmetrics(&formatted);

    if let Err(ref err) = parsed {
        match err {
            ParseError::ParseError(s) => {
                panic!("Parse error:\n{s}")
            }
            ParseError::DuplicateMetric => panic!("Duplicate metric!"),
            ParseError::InvalidMetric(s) => panic!(
                "InvalidMetric:\n\
                {s}\n\
                Note: This parser is very strict (https://github.com/sinkingpoint/openmetrics-parser/issues/12)"
            ),
        }
    }
}
