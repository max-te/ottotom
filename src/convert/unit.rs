/*!
 * OTEL style short unit to Prometheus style long unit conversion
 *
 * Extracted from the [opentelemetry-prometheus crate](https://github.com/open-telemetry/opentelemetry-rust/blob/eac368a7e4addbee3b68c27a0eafae59928ad4c7/opentelemetry-prometheus/src/utils.rs)
 * with modifications.
 * Licensed under the Apache-2.0 License, Original Copyright 2025 The opentelemetry-rust Authors
 */

use std::borrow::Cow;

const NON_APPLICABLE_ON_PER_UNIT: [&str; 8] = ["1", "d", "h", "min", "s", "ms", "us", "ns"];

pub(crate) fn get_unit_suffixes(unit: &str) -> Option<Cow<'static, str>> {
    // no unit return early
    if unit.is_empty() {
        return None;
    }

    // direct match with known units
    if let Some(matched) = get_prom_units(unit) {
        return Some(Cow::Borrowed(matched));
    }

    // converting foo/bar to foo_per_bar
    // split the string by the first '/'
    // if the first part is empty, we just return the second part if it's a match with known per unit
    // e.g
    // "test/y" => "per_year"
    // "km/s" => "kilometers_per_second"
    if let Some((first, second)) = unit.split_once('/') {
        return match (
            NON_APPLICABLE_ON_PER_UNIT.contains(&first),
            get_prom_units(first),
            get_prom_per_unit(second),
        ) {
            (true, _, Some(second_part)) | (false, None, Some(second_part)) => {
                Some(Cow::Owned(format!("per_{second_part}")))
            }
            (false, Some(first_part), Some(second_part)) => {
                Some(Cow::Owned(format!("{first_part}_per_{second_part}")))
            }
            _ => None,
        };
    }

    // Unmatched units and annotations are ignored
    // e.g. "{request}"
    None
}

fn get_prom_units(unit: &str) -> Option<&'static str> {
    match unit {
        // Time
        "d" => Some("days"),
        "h" => Some("hours"),
        "min" => Some("minutes"),
        "s" => Some("seconds"),
        "ms" => Some("milliseconds"),
        "us" => Some("microseconds"),
        "ns" => Some("nanoseconds"),

        // Bytes
        "KiBy" => Some("kibibytes"),
        "MiBy" => Some("mebibytes"),
        "GiBy" => Some("gibibytes"),
        "TiBy" => Some("tibibytes"),
        "By" | "B" => Some("bytes"),
        "KBy" | "KB" => Some("kilobytes"),
        "MBy" | "MB" => Some("megabytes"),
        "GBy" | "GB" => Some("gigabytes"),
        "TBy" | "TB" => Some("terabytes"),

        // SI
        "m" => Some("meters"),
        "V" => Some("volts"),
        "A" => Some("amperes"),
        "J" => Some("joules"),
        "W" => Some("watts"),
        "g" => Some("grams"),

        // Misc
        "Cel" => Some("celsius"),
        "Hz" => Some("hertz"),
        "1" => Some("ratio"),
        "%" => Some("percent"),
        _ => None,
    }
}

fn get_prom_per_unit(unit: &str) -> Option<&'static str> {
    match unit {
        "s" => Some("second"),
        "m" => Some("minute"),
        "h" => Some("hour"),
        "d" => Some("day"),
        "w" => Some("week"),
        "mo" => Some("month"),
        "y" => Some("year"),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_get_unit_suffixes() {
        let test_cases = vec![
            // Direct match
            ("g", Some(Cow::Borrowed("grams"))),
            // Per unit
            ("test/y", Some(Cow::Owned("per_year".to_owned()))),
            ("1/y", Some(Cow::Owned("per_year".to_owned()))),
            ("m/s", Some(Cow::Owned("meters_per_second".to_owned()))),
            // No match
            ("invalid", None),
            ("invalid/invalid", None),
            ("seconds", None),
            ("", None),
            // annotations
            ("{request}", None),
        ];
        for (unit, expected_suffix) in test_cases {
            assert_eq!(get_unit_suffixes(unit), expected_suffix);
        }
    }
}
