//! A plain-data description of what an application records through the
//! OpenTelemetry API. `crate::sdk` replays it into a real SDK pipeline.

use ephyra::{Arbitrary, Gen, GenExt, gens};

/// Characters that exercise name sanitization, escaping and UTF-8 handling.
const TEXT_CHARS: [char; 16] = [
    'a', 'b', '_', '.', '-', '/', ':', '1', ' ', '"', '\\', '\n', '\r', '\0', 'é', '𝄞',
];

/// Characters the SDK accepts after the first letter of an instrument name.
const NAME_TAIL_CHARS: [char; 7] = ['a', 'b', '1', '_', '.', '-', '/'];

/// Instrument names that collide with suffixes the converter appends.
const SUFFIXED_NAMES: [&str; 10] = [
    "a",
    "a_b",
    "a.b",
    "a_total",
    "a_seconds",
    "a_count",
    "a_sum",
    "a_bucket",
    "a_created",
    "a_info",
];

/// Attribute keys that collide with labels the converter adds.
const RESERVED_KEYS: [&str; 5] = ["le", "otel_scope_name", "otel_scope_version", "a.b", "a_b"];

const UNIT_SYMBOLS: [&str; 10] = [
    "", "1", "s", "ms", "By", "{packet}", "m/s", "km/h", "%", "foo",
];

#[ephyra::generator]
pub fn text() -> impl Gen<Value = String> {
    gens::string_len(gens::pick_from(TEXT_CHARS.to_vec()), 0..=8)
}

/// The OpenTelemetry common spec (v1.45.0) requires attribute keys to be
/// non-empty.
#[ephyra::generator]
pub fn is_attribute_key(key: &str) -> bool {
    !key.is_empty()
}

#[ephyra::generator(is_attribute_key)]
pub fn attribute_key() -> impl Gen<Value = String> {
    gens::frequency(vec![
        (
            3,
            gens::string_len(gens::pick_from(TEXT_CHARS.to_vec()), 1..=8).boxed(),
        ),
        (
            1,
            gens::pick_from(RESERVED_KEYS.map(String::from).to_vec()).boxed(),
        ),
    ])
}

/// The rule `opentelemetry_sdk` 0.33 enforces in `validate_instrument_name`.
#[ephyra::generator]
pub fn is_sdk_instrument_name(name: &str) -> bool {
    name.len() <= 255
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-/".contains(c))
}

#[ephyra::generator(is_sdk_instrument_name)]
pub fn instrument_name() -> impl Gen<Value = String> {
    let composed = (
        gens::pick_from(vec!['a', 'b', 'Z']),
        gens::string_len(gens::pick_from(NAME_TAIL_CHARS.to_vec()), 0..=6),
    )
        .iso(
            |(head, tail): (char, String)| format!("{head}{tail}"),
            |name: &String| {
                let mut chars = name.chars();
                let head = chars.next().unwrap_or('a');
                (head, chars.collect())
            },
        );
    gens::frequency(vec![
        (3, composed.boxed()),
        (
            1,
            gens::pick_from(SUFFIXED_NAMES.map(String::from).to_vec()).boxed(),
        ),
    ])
}

#[ephyra::generator]
pub fn units() -> impl Gen<Value = String> {
    gens::pick_from(UNIT_SYMBOLS.map(String::from).to_vec())
}

#[ephyra::generator]
pub fn floats() -> impl Gen<Value = f64> {
    gens::frequency(vec![
        (4, gens::f64_range(-1000.0..=1000.0).boxed()),
        (
            2,
            gens::pick_from(vec![0.0, -0.0, 1.0, 0.5, 1e300, 5e-324, -1.0]).boxed(),
        ),
        (
            1,
            gens::pick_from(vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY]).boxed(),
        ),
        (1, gens::f64_bits().boxed()),
    ])
}

#[ephyra::generator]
pub fn integer() -> impl Gen<Value = i64> {
    gens::frequency(vec![
        (4, gens::int(-100i64..=100).boxed()),
        (1, gens::edges(i64::MIN..=i64::MAX).boxed()),
    ])
}

#[derive(Clone, Debug, PartialEq, Arbitrary)]
pub enum AttrValue {
    Str(#[ephyra(gen = text())] String),
    I64(#[ephyra(gen = integer())] i64),
    F64(#[ephyra(gen = floats())] f64),
    Bool(bool),
}

#[derive(Clone, Debug, PartialEq, Arbitrary)]
pub struct Attr {
    #[ephyra(gen = attribute_key())]
    pub key: String,
    pub value: AttrValue,
}

/// The SDK keys scopes by equality, so a NaN attribute splits one meter into
/// several identical `ScopeMetrics`. That is an SDK quirk, not converter input
/// worth testing.
#[ephyra::generator]
pub fn is_scope_attr(attr: &Attr) -> bool {
    !matches!(attr.value, AttrValue::F64(f) if f.is_nan())
}

#[ephyra::generator(is_scope_attr)]
pub fn scope_attr() -> impl Gen<Value = Attr> {
    ephyra::any::<Attr>().filter("NaN scope attribute", is_scope_attr)
}

#[derive(Clone, Copy, Debug, PartialEq, Arbitrary)]
pub enum Kind {
    U64Counter,
    F64Counter,
    I64UpDownCounter,
    F64UpDownCounter,
    U64Gauge,
    I64Gauge,
    F64Gauge,
    U64Histogram,
    F64Histogram,
}

/// One measurement. The instrument kind decides which number it uses.
#[derive(Clone, Debug, PartialEq, Arbitrary)]
pub struct Record {
    #[ephyra(gen = integer())]
    pub int: i64,
    #[ephyra(gen = floats())]
    pub float: f64,
    #[ephyra(gen = gens::vec(ephyra::any::<Attr>()).len(0..=3))]
    pub attrs: Vec<Attr>,
}

#[derive(Clone, Debug, PartialEq, Arbitrary)]
pub struct Instrument {
    #[ephyra(gen = instrument_name())]
    pub name: String,
    #[ephyra(gen = units())]
    pub unit: String,
    #[ephyra(gen = text())]
    pub description: String,
    pub kind: Kind,
    /// Raw histogram boundaries; `crate::sdk` keeps the finite ones, sorted and deduplicated.
    #[ephyra(gen = gens::option(gens::vec(floats()).len(0..=4)))]
    pub bounds: Option<Vec<f64>>,
    #[ephyra(gen = gens::vec(ephyra::any::<Record>()).len(1..=4))]
    pub records: Vec<Record>,
}

#[derive(Clone, Debug, PartialEq, Arbitrary)]
pub struct Scope {
    #[ephyra(gen = text())]
    pub name: String,
    #[ephyra(gen = gens::option(text()))]
    pub version: Option<String>,
    #[ephyra(gen = gens::vec(scope_attr()).len(0..=2))]
    pub attrs: Vec<Attr>,
    #[ephyra(gen = gens::vec(ephyra::any::<Instrument>()).len(1..=3))]
    pub instruments: Vec<Instrument>,
}

#[derive(Clone, Debug, PartialEq, Arbitrary)]
pub struct Scenario {
    #[ephyra(gen = gens::vec(ephyra::any::<Attr>()).len(0..=2))]
    pub resource: Vec<Attr>,
    #[ephyra(gen = gens::vec(ephyra::any::<Scope>()).len(1..=3))]
    pub scopes: Vec<Scope>,
    pub scope_info: bool,
    pub target_info: bool,
}

#[ephyra::generator(metrics = scenario_metrics)]
pub fn scenario() -> impl Gen<Value = Scenario> {
    ephyra::any::<Scenario>()
}

/// Shape of a scenario: raw counts, so their response to size is visible.
pub fn scenario_metrics(s: &Scenario, _size: usize) -> Vec<(&'static str, f64)> {
    let instruments = s.scopes.iter().flat_map(|scope| &scope.instruments);
    let records: usize = instruments.clone().map(|i| i.records.len()).sum();
    let histograms = instruments
        .clone()
        .filter(|i| matches!(i.kind, Kind::U64Histogram | Kind::F64Histogram))
        .count();
    vec![
        ("scopes", s.scopes.len() as f64),
        ("instruments", instruments.count() as f64),
        ("records", records as f64),
        ("histograms", histograms as f64),
    ]
}
