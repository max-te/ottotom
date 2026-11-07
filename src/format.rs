use ufmt::uDisplay;

pub trait Numeric {
    fn fast_display(&self) -> impl uDisplay + Copy + use<Self>;
    fn is_unsigned() -> bool;
    fn is_nonnegative(&self) -> bool;
    /// Converts the value to `f64`, for comparing metric values against
    /// histogram bucket bounds (which the SDK always stores as `f64`).
    fn to_f64(&self) -> f64;
}

#[derive(Copy, Clone)]
struct RyuDisplay(f64);

impl uDisplay for RyuDisplay {
    fn fmt<W>(&self, f: &mut ufmt::Formatter<'_, W>) -> Result<(), W::Error>
    where
        W: ufmt::uWrite + ?Sized,
    {
        // om[impl numbers.canonical-inf]
        if self.0.is_infinite() {
            f.write_char(if self.0.is_sign_positive() { '+' } else { '-' })?;
            f.write_str("Inf")
        } else {
            let mut buffer = ryu::Buffer::new();
            let formatted = buffer.format(self.0);

            f.write_str(formatted)
        }
    }
}

/// Displays `value` as a canonical OpenMetrics number, as `le` label values
/// require: Go's shortest `%g` rendering, with `.0` appended when it has
/// neither a decimal point nor an exponent.
pub(crate) fn canonical_display(value: f64) -> impl uDisplay + Copy {
    CanonicalDisplay(value)
}

#[derive(Copy, Clone)]
struct CanonicalDisplay(f64);

impl uDisplay for CanonicalDisplay {
    fn fmt<W>(&self, f: &mut ufmt::Formatter<'_, W>) -> Result<(), W::Error>
    where
        W: ufmt::uWrite + ?Sized,
    {
        // In this range, and for non-finite values, ryu already writes the
        // canonical form: fixed notation with at least one decimal.
        if !self.0.is_finite() || self.0 == 0.0 || (1e-4..1e6).contains(&self.0.abs()) {
            return RyuDisplay(self.0).fmt(f);
        }
        let mut buffer = ryu::Buffer::new();
        let shortest = Shortest::read(buffer.format_finite(self.0));
        let mut out = Out::default();
        shortest.write_canonical(&mut out);
        f.write_str(out.as_str())
    }
}

/// The shortest digits of a finite number, read back from ryu's output. The
/// number is `0.DIGITS × 10^point`, negated if `negative`.
struct Shortest {
    negative: bool,
    /// significant digits as ASCII, without leading or trailing zeros
    digits: [u8; 24],
    len: usize,
    point: i32,
}

impl Shortest {
    /// Reads ryu's output, which is `[-]` followed by `DDD.0`, `DD.DD`,
    /// `0.000DD`, `DeE` or `D.DDDeE`, where the exponent `E` may be negative.
    fn read(text: &str) -> Self {
        let text = text.as_bytes();
        let (negative, text) = match text.split_first() {
            Some((b'-', rest)) => (true, rest),
            _ => (false, text),
        };
        let (mantissa, exponent) = match text.iter().position(|&b| b == b'e') {
            Some(e) => {
                let exponent = std::str::from_utf8(&text[e + 1..])
                    .ok()
                    .and_then(|exponent| exponent.parse::<i32>().ok())
                    .expect("ryu writes a decimal exponent");
                (&text[..e], exponent)
            }
            None => (text, 0),
        };

        let mut shortest = Shortest {
            negative,
            digits: [0; 24],
            len: 0,
            point: exponent,
        };
        let mut after_point = false;
        for &byte in mantissa {
            if byte == b'.' {
                after_point = true;
            } else if shortest.len == 0 && byte == b'0' {
                // A leading zero after the point moves the point right.
                if after_point {
                    shortest.point -= 1;
                }
            } else {
                shortest.digits[shortest.len] = byte;
                shortest.len += 1;
                if !after_point {
                    shortest.point += 1;
                }
            }
        }
        while shortest.len > 0 && shortest.digits[shortest.len - 1] == b'0' {
            shortest.len -= 1;
        }
        shortest
    }

    /// Writes the number as Go's shortest `%g` does: with an exponent if the
    /// decimal exponent is below -4 or at least 6, in fixed notation otherwise.
    fn write_canonical(&self, out: &mut Out) {
        if self.negative {
            out.push(b'-');
        }
        let digits = &self.digits[..self.len];
        if digits.is_empty() {
            out.extend(b"0.0");
            return;
        }
        let exponent = self.point - 1;
        if !(-4..6).contains(&exponent) {
            out.push(digits[0]);
            if digits.len() > 1 {
                out.push(b'.');
                out.extend(&digits[1..]);
            }
            out.push(b'e');
            out.push(if exponent < 0 { b'-' } else { b'+' });
            let exponent = exponent.unsigned_abs();
            if exponent >= 100 {
                out.push(digit(exponent / 100));
            }
            out.push(digit(exponent / 10 % 10));
            out.push(digit(exponent % 10));
            return;
        }
        // `point` is between -3 and 6 here.
        let point = usize::try_from(self.point.max(0)).expect("the point is not negative");
        if self.point <= 0 {
            out.extend(b"0.");
            for _ in self.point..0 {
                out.push(b'0');
            }
            out.extend(digits);
        } else if point >= digits.len() {
            out.extend(digits);
            for _ in digits.len()..point {
                out.push(b'0');
            }
            out.extend(b".0");
        } else {
            out.extend(&digits[..point]);
            out.push(b'.');
            out.extend(&digits[point..]);
        }
    }
}

/// The ASCII digit for `value`, which is below 10.
fn digit(value: u32) -> u8 {
    b'0' + u8::try_from(value).expect("a single digit")
}

/// A stack buffer for one canonical number, which takes at most 24 bytes,
/// as in `-1.2345678901234567e-308`.
#[derive(Default)]
struct Out {
    bytes: [u8; 32],
    len: usize,
}

impl Out {
    fn push(&mut self, byte: u8) {
        self.bytes[self.len] = byte;
        self.len += 1;
    }

    fn extend(&mut self, bytes: &[u8]) {
        self.bytes[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).expect("only ASCII is written")
    }
}

// om[impl numbers.float]
impl Numeric for f64 {
    #[inline]
    fn fast_display(&self) -> impl uDisplay + Copy + use<> {
        RyuDisplay(*self)
    }

    #[inline]
    fn is_unsigned() -> bool {
        false
    }

    #[inline]
    fn is_nonnegative(&self) -> bool {
        self.is_sign_positive()
    }

    #[inline]
    fn to_f64(&self) -> f64 {
        *self
    }
}

#[cfg(test)]
mod test_float_format {
    use ufmt::uwrite;

    use crate::format::Numeric;

    #[test]
    fn has_decimal_point() {
        let mut s = String::new();
        uwrite!(s, "{}", 5.0f64.fast_display()).unwrap();
        assert_eq!(s, "5.0");
    }

    #[test]
    fn has_decimals() {
        let mut s = String::new();
        uwrite!(s, "{}", 0.12345f64.fast_display()).unwrap();
        assert_eq!(s, "0.12345");
    }

    #[test]
    fn has_scientific_notation() {
        let mut s = String::new();
        uwrite!(s, "{}", 0.000_000_000_012_f64.fast_display()).unwrap();
        assert_eq!(s, "1.2e-11");
    }

    #[test]
    fn has_inf() {
        let mut s = String::new();
        uwrite!(s, "{}", f64::INFINITY.fast_display()).unwrap();
        // om[verify numbers.canonical-inf]
        assert_eq!(s, "+Inf");
        s.clear();
        uwrite!(s, "{}", f64::NEG_INFINITY.fast_display()).unwrap();
        assert_eq!(s, "-Inf");
    }

    #[test]
    fn has_nan() {
        let mut s = String::new();
        uwrite!(s, "{}", f64::NAN.fast_display()).unwrap();
        assert_eq!(s, "NaN");
    }
}

#[cfg(test)]
mod test_canonical_format {
    use ufmt::uwrite;

    use crate::format::canonical_display;

    fn canonical(value: f64) -> String {
        let mut s = String::new();
        uwrite!(s, "{}", canonical_display(value)).unwrap();
        s
    }

    #[test]
    // om[verify histogram.sorted] - the examples of "Considerations: Canonical Numbers"
    fn renders_the_spec_examples() {
        let examples = [
            "0.0", "0.001", "0.002", "0.01", "0.1", "0.9", "0.95", "0.99", "0.999", "1.0", "1.7",
            "10.0", "1e-10", "1e-09", "1e-05", "0.0001", "0.1", "1.0", "100000.0", "1e+06",
            "1e+10",
        ];
        for example in examples {
            assert_eq!(canonical(example.parse().unwrap()), example);
        }
    }

    #[test]
    fn renders_signs_and_non_finite_values() {
        assert_eq!(canonical(-0.0), "-0.0");
        assert_eq!(canonical(-2.5), "-2.5");
        assert_eq!(canonical(-1e6), "-1e+06");
        assert_eq!(canonical(5e-324), "5e-324");
        assert_eq!(canonical(f64::MAX), "1.7976931348623157e+308");
        // om[verify numbers.canonical-inf]
        assert_eq!(canonical(f64::INFINITY), "+Inf");
        assert_eq!(canonical(f64::NEG_INFINITY), "-Inf");
        assert_eq!(canonical(f64::NAN), "NaN");
    }

    #[test]
    // om[verify histogram.sorted]
    fn matches_go() {
        // Generated by tests/data/canonical_le.go. Each line is the shortest
        // rendering of its value, so parsing it gives back that value.
        let reference = include_str!("../tests/data/canonical_le.txt");
        for expected in reference.lines() {
            let value: f64 = expected.parse().unwrap();
            assert_eq!(canonical(value), expected, "{value:e}");
        }
    }
}

#[derive(Copy, Clone)]
struct ItoaDisplay<N: itoa::Integer>(N);

impl<N: itoa::Integer> uDisplay for ItoaDisplay<N> {
    fn fmt<W>(&self, f: &mut ufmt::Formatter<'_, W>) -> Result<(), W::Error>
    where
        W: ufmt::uWrite + ?Sized,
    {
        let mut buffer = itoa::Buffer::new();
        let formatted = buffer.format(self.0);
        f.write_str(formatted)
    }
}

// om[impl numbers.integer]
impl Numeric for u64 {
    #[inline]
    fn fast_display(&self) -> impl uDisplay + Copy + use<> {
        ItoaDisplay(*self)
    }

    #[inline]
    fn is_unsigned() -> bool {
        true
    }

    #[inline]
    fn is_nonnegative(&self) -> bool {
        true
    }

    #[inline]
    fn to_f64(&self) -> f64 {
        *self as f64
    }
}

// om[impl numbers.integer]
impl Numeric for i64 {
    #[inline]
    fn fast_display(&self) -> impl uDisplay + Copy + use<> {
        ItoaDisplay(*self)
    }

    #[inline]
    fn is_unsigned() -> bool {
        false
    }

    #[inline]
    fn is_nonnegative(&self) -> bool {
        !self.is_negative()
    }

    #[inline]
    fn to_f64(&self) -> f64 {
        *self as f64
    }
}
