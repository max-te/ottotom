//! Writing names and label sets: sanitizing, escaping and hashing attributes.

use std::fmt::Write;
use std::hash::{DefaultHasher, Hasher};

use opentelemetry::{Array, Key, KeyValue, Value};
use ufmt::uWrite;

/// Whether `key` is written as the label name `label`, compared without allocating.
pub(super) fn sanitizes_to(key: &str, label: &str) -> bool {
    /// Accepts exactly the text of the label, piece by piece.
    struct Expect<'l>(&'l str);

    impl uWrite for Expect<'_> {
        type Error = ();

        fn write_str(&mut self, s: &str) -> Result<(), ()> {
            self.0 = self.0.strip_prefix(s).ok_or(())?;
            Ok(())
        }
    }

    // Sanitizing keeps a leading letter, and every reserved label starts with one.
    if key.as_bytes().first() != label.as_bytes().first() {
        return false;
    }
    let mut expect = Expect(label);
    write_sanitized_name(&mut expect, key, NameKind::AttributeLabel).is_ok() && expect.0.is_empty()
}

/// Write the attribute string for attrs. Does not write curly braces.
pub(super) fn write_attrs<'a, I: Iterator<Item = &'a KeyValue>, U: uWrite>(
    f: &mut U,
    attrs: I,
) -> Result<(), U::Error> {
    write_attrs_with(f, &mut LabelScratch::default(), attrs)
}

/// Like [`write_attrs`], but reuses `scratch`, so the per-point writers
/// allocate nothing new.
pub(super) fn write_attrs_with<'a, I: Iterator<Item = &'a KeyValue>, U: uWrite>(
    f: &mut U,
    scratch: &mut LabelScratch,
    attrs: I,
) -> Result<(), U::Error> {
    write_attrs_tuple(f, scratch, attrs.map(|kv| (&kv.key, &kv.value)))
}

/// One label while a label set is written: the byte range of its sanitized
/// name in [`LabelScratch::names`], its original key and its value.
type Label<'a> = (usize, usize, &'a Key, &'a Value);

/// Reusable buffers for writing one label set.
#[derive(Default)]
pub(super) struct LabelScratch {
    /// the sanitized label names, back to back
    names: String,
    /// the memory of a `Vec<Label>`, kept empty between label sets
    labels: Vec<(usize, usize, usize, usize)>,
}

// The label list reuses the memory of `LabelScratch::labels`, so both
// element types must have the same layout.
const _: () = {
    assert!(size_of::<Label<'static>>() == size_of::<(usize, usize, usize, usize)>());
    assert!(align_of::<Label<'static>>() == align_of::<(usize, usize, usize, usize)>());
};

/// Turns an empty `Vec<T>` into an empty `Vec<V>`. The standard library
/// collects a mapped `vec::IntoIter` in place when `T` and `V` have the same
/// layout, so the memory is reused and nothing is allocated.
fn recycle<T, V>(mut vec: Vec<T>) -> Vec<V> {
    vec.clear();
    vec.into_iter()
        .map(|_| -> V { unreachable!("the vector is empty") })
        .collect()
}

pub(super) fn write_attrs_tuple<'a, I: Iterator<Item = (&'a Key, &'a Value)>, U: uWrite>(
    f: &mut U,
    scratch: &mut LabelScratch,
    attrs: I,
) -> Result<(), U::Error> {
    // c[impl mattrs.to-labels] - attribute keys/values are emitted as label name/value pairs
    // Each key is sanitized once into `label_names`; a label holds its byte range there.
    let label_names = &mut scratch.names;
    label_names.clear();
    let mut labels: Vec<Label<'a>> = recycle(std::mem::take(&mut scratch.labels));
    labels.extend(attrs.map(|(key, value)| {
        let start = label_names.len();
        let Ok(()) = write_sanitized_name(label_names, key.as_str(), NameKind::AttributeLabel);
        (start, label_names.len(), key, value)
    }));
    // c[impl mattrs.key-sanitize.collisions] - colliding labels become adjacent,
    // in the order of their original keys
    labels.sort_unstable_by(|a, b| {
        label_names[a.0..a.1]
            .cmp(&label_names[b.0..b.1])
            .then_with(|| a.2.cmp(b.2))
    });

    let mut previous_name: Option<&str> = None;
    for &(start, end, _, value) in &labels {
        let name = &label_names[start..end];
        if previous_name == Some(name) {
            // c[impl mattrs.key-sanitize.collisions] - values of one label are joined with `;`
            f.write_char(';')?;
        } else {
            if previous_name.is_some() {
                f.write_str("\",")?;
            }
            f.write_str(name)?;
            f.write_str("=\"")?;
        }
        // c[impl mattrs.type-conversion] - non-string values are stringified
        // c[impl attrs.stringify] - as opentelemetry-sdk's `Value::as_str` does
        write_escaped_value(f, value)?;
        previous_name = Some(name);
    }
    if previous_name.is_some() {
        f.write_char('"')?;
    }
    scratch.labels = recycle(labels);
    Ok(())
}

/// Calculates a hash of the [`KeyValue`] pairs which is invariant under reordering of the [`KeyValue`]s within the [`Iterator`].
pub(super) fn hash_attrs<'a, I: Iterator<Item = &'a KeyValue>>(attrs: I) -> u64 {
    let mut hash = 0;
    for kv in attrs {
        let mut hasher = DefaultHasher::default();
        hasher.write(kv.key.as_str().as_bytes());
        // The type tells apart values that stringify alike, such as 1 and "1".
        hasher.write_u8(value_type(&kv.value));
        write!(HashWriter(&mut hasher), "{}", kv.value).expect("hashing cannot fail");
        hash ^= hasher.finish(); // XOR to be order-invariant
    }
    hash
}

/// Feeds formatted text into a hasher. [`DefaultHasher::write`] hashes a stream
/// of bytes, so the pieces hash like the whole text.
struct HashWriter<'h>(&'h mut DefaultHasher);

impl Write for HashWriter<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.write(s.as_bytes());
        Ok(())
    }
}

/// A number for each type of attribute value.
fn value_type(value: &Value) -> u8 {
    match value {
        Value::Bool(_) => 0,
        Value::I64(_) => 1,
        Value::F64(_) => 2,
        Value::String(_) => 3,
        Value::Array(Array::Bool(_)) => 4,
        Value::Array(Array::I64(_)) => 5,
        Value::Array(Array::F64(_)) => 6,
        Value::Array(Array::String(_)) => 7,
        _ => 8,
    }
}

/// Writes `value` escaped, as [`Value::as_str`] renders it, but without
/// allocating: non-string values are formatted straight into `f`.
pub(super) fn write_escaped_value<U: uWrite>(f: &mut U, value: &Value) -> Result<(), U::Error> {
    if let Value::String(value) = value {
        return write_escaped(f, value.as_str());
    }
    let mut writer = EscapingWriter { f, error: None };
    match write!(writer, "{value}") {
        Ok(()) => Ok(()),
        Err(std::fmt::Error) => Err(writer.error.expect("only the output can fail")),
    }
}

/// Escapes formatted text into a [`uWrite`]. It keeps the output's error,
/// because [`Write`] can only report [`std::fmt::Error`].
struct EscapingWriter<'f, U: uWrite> {
    f: &'f mut U,
    error: Option<U::Error>,
}

impl<U: uWrite> Write for EscapingWriter<'_, U> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        write_escaped(self.f, s).map_err(|error| {
            self.error = Some(error);
            std::fmt::Error
        })
    }
}

/// Writes to `f` the contents of `value` as an escaped string. Does not put quotes around the value.
/// The chars to escape are `\`, `"` and `\n`.
// om[impl escaping.chars]
pub(super) fn write_escaped<U: uWrite>(f: &mut U, value: &str) -> Result<(), U::Error> {
    #[inline]
    fn next_escape_char(bytes: &[u8]) -> Option<usize> {
        memchr::memchr3(b'\\', b'"', b'\n', bytes)
    }

    let mut bytes = value.as_bytes();

    while let Some(next_escape) = next_escape_char(bytes) {
        let (head, tail) = bytes.split_at(next_escape);
        f.write_str(str::from_utf8(head).expect("escapable chars should be on a char boundary"))?;
        match tail[0] {
            b'\\' => f.write_str("\\\\"),
            b'"' => f.write_str("\\\""),
            b'\n' => f.write_str("\\n"),
            _ => unreachable!("next_escape_char should find one of the 3 escapable chars"),
        }?;
        bytes = &tail[1..];
    }
    // om[impl strings.utf8] - strings are written as valid UTF-8
    f.write_str(str::from_utf8(bytes).expect("escaped string should be valid utf-8"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NameKind {
    Metric,
    AttributeLabel,
}

/// Write `name` as an OpenMetrics metrics name, replacing any illegal characters with underscore according to the
/// [spec](https://github.com/open-telemetry/opentelemetry-specification/blob/v1.45.0/specification/compatibility/prometheus_and_openmetrics.md#metric-metadata-1).
// c[impl metadata.name-sanitize]
// c[impl mattrs.key-sanitize]
// om[related labels.reserved] - a leading illegal character is sanitized to
// a `_` prefix (e.g. `1.label` -> `_1_label`), producing an underscore-leading
// label name that technically violates om[labels.reserved]; accepted deviation,
// matching the opentelemetry-prometheus reference impl.
pub(super) fn write_sanitized_name<U: uWrite>(
    f: &mut U,
    name: &str,
    kind: NameKind,
) -> Result<(), U::Error> {
    // Multiple consecutive `_` characters MUST be replaced with a single `_` character
    let mut previous_was_underscore = false;
    // The name must not start with a digit
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        f.write_char('_')?;
        previous_was_underscore = true;
    }
    // Allowed characters are written in runs, one `write_str` per run. They are
    // all ASCII, so scanning bytes is enough: the bytes of a multi-byte char are
    // all invalid and collapse into one `_`, and every run boundary sits next to
    // an ASCII byte, which is always a char boundary.
    let mut run_start = 0;
    for (i, &byte) in name.as_bytes().iter().enumerate() {
        // Allowed characters are `a-z A-Z 0-9 : _`, except for ':' in labels
        // Invalid characters in the metric name MUST be replaced with the `_` character.
        if byte.is_ascii_alphanumeric() || (byte == b':' && kind == NameKind::Metric) {
            if previous_was_underscore {
                run_start = i;
                previous_was_underscore = false;
            }
        } else if !previous_was_underscore {
            f.write_str(&name[run_start..i])?;
            f.write_char('_')?;
            previous_was_underscore = true;
        }
    }
    if !previous_was_underscore {
        f.write_str(&name[run_start..])?;
    }
    Ok(())
}
