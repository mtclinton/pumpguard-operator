//! Colored logging module for PumpGuard

use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialize the tracing logger with colored output
pub fn init_logger() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,pumpguard=debug"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_target(true)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false)
                .with_ansi(true),
        )
        .init();
}

/// Log macros with module prefixes and emojis
#[macro_export]
macro_rules! log_info {
    ($module:expr, $($arg:tt)*) => {
        tracing::info!(target: $module, $($arg)*)
    };
}

#[macro_export]
macro_rules! log_success {
    ($module:expr, $($arg:tt)*) => {
        tracing::info!(target: $module, "✅ {}", format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($module:expr, $($arg:tt)*) => {
        tracing::warn!(target: $module, $($arg)*)
    };
}

#[macro_export]
macro_rules! log_error {
    ($module:expr, $($arg:tt)*) => {
        tracing::error!(target: $module, $($arg)*)
    };
}

#[macro_export]
macro_rules! log_whale {
    ($($arg:tt)*) => {
        tracing::info!(target: "WHALE", "🐋 {}", format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_rug {
    ($($arg:tt)*) => {
        tracing::error!(target: "RUG_ALERT", "🚨 {}", format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_token {
    ($($arg:tt)*) => {
        tracing::info!(target: "TOKEN", "🆕 {}", format!($($arg)*))
    };
}

