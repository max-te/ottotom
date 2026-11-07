//! A small reader for the OpenMetrics text that ottotom writes. It
//! understands only the line shapes ottotom produces, and reports anything
//! else as an error, so the properties do not depend on a third-party parser.

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Type { family: String, typ: String },
    Unit { family: String, unit: String },
    Help { family: String, text: String },
    Sample(Sample),
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub name: String,
    /// Label names and unescaped values, in output order.
    pub labels: Vec<(String, String)>,
    pub value: String,
    pub timestamp: Option<String>,
    pub exemplar: Option<Exemplar>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Exemplar {
    pub labels: Vec<(String, String)>,
    pub value: String,
    pub timestamp: Option<String>,
}

impl Sample {
    pub fn label(&self, name: &str) -> Option<&str> {
        self.labels
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Reads the whole exposition. The text must end with `# EOF\n`.
pub fn parse(text: &str) -> Result<Vec<Line>, String> {
    let body = text
        .strip_suffix('\n')
        .ok_or("the exposition does not end with a newline")?;
    let mut lines = Vec::new();
    for (number, line) in body.split('\n').enumerate() {
        let parsed = parse_line(line).map_err(|e| format!("line {}: {e}: {line:?}", number + 1))?;
        lines.push(parsed);
    }
    match lines.last() {
        Some(Line::Eof) => {}
        _ => return Err("the last line is not `# EOF`".into()),
    }
    if lines.iter().filter(|l| **l == Line::Eof).count() != 1 {
        return Err("`# EOF` appears more than once".into());
    }
    Ok(lines)
}

fn parse_line(line: &str) -> Result<Line, String> {
    if line == "# EOF" {
        return Ok(Line::Eof);
    }
    if let Some(rest) = line.strip_prefix("# TYPE ") {
        let (family, typ) = rest.split_once(' ').ok_or("TYPE without a type")?;
        return Ok(Line::Type {
            family: family.into(),
            typ: typ.into(),
        });
    }
    if let Some(rest) = line.strip_prefix("# UNIT ") {
        let (family, unit) = rest.split_once(' ').ok_or("UNIT without a unit")?;
        return Ok(Line::Unit {
            family: family.into(),
            unit: unit.into(),
        });
    }
    if let Some(rest) = line.strip_prefix("# HELP ") {
        let (family, escaped) = rest.split_once(' ').ok_or("HELP without text")?;
        let mut chars = escaped.chars().peekable();
        let text = read_escaped(&mut chars, None)?;
        return Ok(Line::Help {
            family: family.into(),
            text,
        });
    }
    if line.starts_with('#') {
        return Err("unknown comment line".into());
    }
    parse_sample(line).map(Line::Sample)
}

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

fn parse_sample(line: &str) -> Result<Sample, String> {
    let mut chars = line.chars().peekable();
    let mut name = String::new();
    while let Some(&c) = chars.peek() {
        if c == '{' || c == ' ' {
            break;
        }
        name.push(c);
        chars.next();
    }
    let labels = if chars.peek() == Some(&'{') {
        read_labels(&mut chars)?
    } else {
        Vec::new()
    };
    expect(&mut chars, ' ')?;
    let value = read_token(&mut chars);
    let mut timestamp = None;
    let mut exemplar = None;
    if chars.peek() == Some(&' ') {
        chars.next();
        if chars.peek() == Some(&'#') {
            exemplar = Some(read_exemplar(&mut chars)?);
        } else {
            timestamp = Some(read_token(&mut chars));
            if chars.peek() == Some(&' ') {
                chars.next();
                exemplar = Some(read_exemplar(&mut chars)?);
            }
        }
    }
    if let Some(rest) = chars.next() {
        return Err(format!("unexpected {rest:?} after the sample"));
    }
    Ok(Sample {
        name,
        labels,
        value,
        timestamp,
        exemplar,
    })
}

fn read_exemplar(chars: &mut Chars<'_>) -> Result<Exemplar, String> {
    expect(chars, '#')?;
    expect(chars, ' ')?;
    let labels = read_labels(chars)?;
    expect(chars, ' ')?;
    let value = read_token(chars);
    let mut timestamp = None;
    if chars.peek() == Some(&' ') {
        chars.next();
        timestamp = Some(read_token(chars));
    }
    Ok(Exemplar {
        labels,
        value,
        timestamp,
    })
}

fn read_labels(chars: &mut Chars<'_>) -> Result<Vec<(String, String)>, String> {
    expect(chars, '{')?;
    let mut labels = Vec::new();
    if chars.peek() == Some(&'}') {
        chars.next();
        return Ok(labels);
    }
    loop {
        let mut name = String::new();
        loop {
            match chars.next() {
                Some('=') => break,
                Some(c) => name.push(c),
                None => return Err("label name without `=`".into()),
            }
        }
        expect(chars, '"')?;
        let value = read_escaped(chars, Some('"'))?;
        labels.push((name, value));
        match chars.next() {
            Some(',') => {}
            Some('}') => return Ok(labels),
            other => {
                return Err(format!(
                    "expected `,` or `}}` after a label, found {other:?}"
                ));
            }
        }
    }
}

/// Reads an escaped string up to `end` (consumed), or to the end of input.
fn read_escaped(chars: &mut Chars<'_>, end: Option<char>) -> Result<String, String> {
    let mut out = String::new();
    loop {
        match chars.next() {
            None if end.is_none() => return Ok(out),
            None => return Err("unterminated string".into()),
            Some(c) if Some(c) == end => return Ok(out),
            Some('\\') => match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('n') => out.push('\n'),
                other => return Err(format!("invalid escape {other:?}")),
            },
            Some(c) => out.push(c),
        }
    }
}

fn read_token(chars: &mut Chars<'_>) -> String {
    let mut token = String::new();
    while let Some(&c) = chars.peek() {
        if c == ' ' {
            break;
        }
        token.push(c);
        chars.next();
    }
    token
}

fn expect(chars: &mut Chars<'_>, wanted: char) -> Result<(), String> {
    match chars.next() {
        Some(c) if c == wanted => Ok(()),
        other => Err(format!("expected {wanted:?}, found {other:?}")),
    }
}

/// A sample together with the family it belongs to.
#[derive(Clone, Debug)]
pub struct FamilySample<'a> {
    pub family: &'a str,
    pub typ: &'a str,
    pub sample: &'a Sample,
}

/// Pairs each sample with the most recent TYPE line before it.
pub fn samples_by_family(lines: &[Line]) -> Vec<FamilySample<'_>> {
    let mut current: Option<(&str, &str)> = None;
    let mut out = Vec::new();
    for line in lines {
        match line {
            Line::Type { family, typ } => current = Some((family, typ)),
            Line::Sample(sample) => {
                let (family, typ) = current.unwrap_or(("", "unknown"));
                out.push(FamilySample {
                    family,
                    typ,
                    sample,
                });
            }
            _ => {}
        }
    }
    out
}

/// The sample names a family of type `typ` named `family` may produce.
pub fn sample_names(family: &str, typ: &str) -> Vec<String> {
    let suffixes: &[&str] = match typ {
        "counter" => &["_total", "_created"],
        "histogram" => &["_count", "_sum", "_bucket", "_created"],
        "info" => &["_info"],
        _ => &[""],
    };
    suffixes.iter().map(|s| format!("{family}{s}")).collect()
}

/// A metric value read back as either an integer or a float.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Number {
    Int(i128),
    /// The bit pattern, with every NaN mapped to one pattern.
    Float(u64),
}

impl Number {
    pub fn from_f64(value: f64) -> Self {
        if value.is_nan() {
            Number::Float(f64::NAN.to_bits())
        } else {
            Number::Float(value.to_bits())
        }
    }

    /// Reads OpenMetrics number text; integers have no `.`, `e`, `Inf` or `NaN`.
    pub fn parse(text: &str) -> Option<Self> {
        if let Ok(int) = text.parse::<i128>() {
            return Some(Number::Int(int));
        }
        match text {
            "+Inf" => Some(Self::from_f64(f64::INFINITY)),
            "-Inf" => Some(Self::from_f64(f64::NEG_INFINITY)),
            "NaN" => Some(Self::from_f64(f64::NAN)),
            _ => {
                let looks_numeric = text
                    .chars()
                    .all(|c| c.is_ascii_digit() || "+-.e".contains(c));
                looks_numeric
                    .then(|| text.parse::<f64>().ok().map(Self::from_f64))
                    .flatten()
            }
        }
    }
}
