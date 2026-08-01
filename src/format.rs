use ufmt::uDisplay;

pub trait FastDisplay {
    fn fast_display(&self) -> impl uDisplay + Copy + use<Self>;
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

// om[impl numbers.float]
impl FastDisplay for f64 {
    #[inline]
    fn fast_display(&self) -> impl uDisplay + Copy + use<> {
        RyuDisplay(*self)
    }
}

#[cfg(test)]
mod test_float_format {
    use ufmt::uwrite;

    use crate::format::FastDisplay;

    #[test]
    fn has_decimal_point() {
        let mut s = String::new();
        uwrite!(s, "{}", 5.0f64.fast_display()).unwrap();
        assert_eq!(s, "5.0")
    }

    #[test]
    fn has_decimals() {
        let mut s = String::new();
        uwrite!(s, "{}", 0.12345f64.fast_display()).unwrap();
        assert_eq!(s, "0.12345")
    }

    #[test]
    fn has_scientific_notation() {
        let mut s = String::new();
        uwrite!(s, "{}", 0.000000000012f64.fast_display()).unwrap();
        assert_eq!(s, "1.2e-11")
    }

    #[test]
    fn has_inf() {
        let mut s = String::new();
        uwrite!(s, "{}", f64::INFINITY.fast_display()).unwrap();
        // om[verify numbers.canonical-inf]
        assert_eq!(s, "+Inf");
        s.clear();
        uwrite!(s, "{}", f64::NEG_INFINITY.fast_display()).unwrap();
        assert_eq!(s, "-Inf")
    }

    #[test]
    fn has_nan() {
        let mut s = String::new();
        uwrite!(s, "{}", f64::NAN.fast_display()).unwrap();
        assert_eq!(s, "NaN");
    }
}

#[cfg(feature = "fast")]
mod fast_impl_with {
    use ufmt::uDisplay;

    use super::FastDisplay;
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

    impl FastDisplay for u64 {
        #[inline]
        fn fast_display(&self) -> impl uDisplay + Copy + use<> {
            ItoaDisplay(*self)
        }
    }

    impl FastDisplay for i64 {
        #[inline]
        fn fast_display(&self) -> impl uDisplay + Copy + use<> {
            ItoaDisplay(*self)
        }
    }
}
#[cfg(not(feature = "fast"))]
mod fast_impl_without {
    use super::FastDisplay;
    use ufmt::uDisplay;

    impl FastDisplay for u64 {
        #[inline]
        fn fast_display(&self) -> impl uDisplay + Copy + use<> {
            *self
        }
    }

    impl FastDisplay for i64 {
        #[inline]
        fn fast_display(&self) -> impl uDisplay + Copy + use<> {
            *self
        }
    }
}
