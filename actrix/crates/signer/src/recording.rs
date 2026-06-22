//! KS-local log macros used to block direct `tracing::*!` callsites.
//!
//! KS cannot depend on `platform::recording` due dependency direction.
//! These macros provide a migration facade with explicit namespaced usage.

#[macro_export]
macro_rules! ks_recording_trace {
    ($($arg:tt)+) => {{
        tracing::event!(tracing::Level::TRACE, "{}", format!($($arg)+));
    }};
}

#[macro_export]
macro_rules! ks_recording_debug {
    ($($arg:tt)+) => {{
        tracing::event!(tracing::Level::DEBUG, "{}", format!($($arg)+));
    }};
}

#[macro_export]
macro_rules! ks_recording_info {
    ($($arg:tt)+) => {{
        tracing::event!(tracing::Level::INFO, "{}", format!($($arg)+));
    }};
}

#[macro_export]
macro_rules! ks_recording_warn {
    ($($arg:tt)+) => {{
        tracing::event!(tracing::Level::WARN, "{}", format!($($arg)+));
    }};
}

#[macro_export]
macro_rules! ks_recording_error {
    ($($arg:tt)+) => {{
        tracing::event!(tracing::Level::ERROR, "{}", format!($($arg)+));
    }};
}

pub use crate::ks_recording_debug as debug;
pub use crate::ks_recording_error as error;
pub use crate::ks_recording_info as info;
pub use crate::ks_recording_trace as trace;
pub use crate::ks_recording_warn as warn;
