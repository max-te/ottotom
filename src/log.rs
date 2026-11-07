//! Internal logging shims.
//!
//! The `tracing` dependency is optional, so every log call needs a
//! `#[cfg(feature = "tracing")]`. Forgetting one only breaks the feature
//! combinations that disable `tracing`, which is easy to miss. These macros move
//! the `cfg` to a single place: with the feature off they expand to a no-op that
//! still type-checks its arguments, so a stale format string or a moved variable
//! is caught in every feature combination.
//!
//! A warning that drops data points reports their number in a leading
//! `points = n` field, so that dropped points can be counted.
//!
//! Which of these are used depends on the enabled features (`error!`/`debug!`
//! only exist on call sites inside the `exporter` module), so an unused macro
//! here is expected rather than dead code.

#![allow(unused_macros)]

#[cfg(feature = "tracing")]
macro_rules! error {
    ($($arg:tt)*) => { ::tracing::error!($($arg)*) };
}

#[cfg(not(feature = "tracing"))]
macro_rules! error {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }};
}

#[cfg(feature = "tracing")]
macro_rules! warn {
    ($($arg:tt)*) => { ::tracing::warn!($($arg)*) };
}

#[cfg(not(feature = "tracing"))]
macro_rules! warn {
    ($field:ident = $value:expr, $($arg:tt)*) => {{
        let _ = &$value;
        let _ = format_args!($($arg)*);
    }};
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }};
}

#[cfg(feature = "tracing")]
macro_rules! debug {
    ($($arg:tt)*) => { ::tracing::debug!($($arg)*) };
}

#[cfg(not(feature = "tracing"))]
macro_rules! debug {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }};
}
