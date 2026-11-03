//! 极简日志：靠 `NIRI_DOCK_LOG` 控制级别，默认只报 warn 及以上。
//!
//! 级别（不区分大小写）：`off` / `error` / `warn` / `info` / `debug`，默认 `info`。
//! 排查时用 `NIRI_DOCK_LOG=debug niri-dock restart`，输出会写进
//! `~/.local/state/niri-dock/dock.log`（`niri-dock logs` 可以直接看）。

use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Level {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
            Level::Debug => "debug",
        }
    }

    fn from_env() -> Level {
        match std::env::var("NIRI_DOCK_LOG")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "off" | "none" | "0" | "false" => Level::Off,
            "error" => Level::Error,
            "info" => Level::Info,
            "debug" | "trace" => Level::Debug,
            "warn" | "warning" => Level::Warn,
            _ => Level::Info,
        }
    }
}

fn level() -> Level {
    static LEVEL: OnceLock<Level> = OnceLock::new();
    *LEVEL.get_or_init(Level::from_env)
}

pub(crate) fn enabled(level: Level) -> bool {
    level <= self::level()
}

/// 日志文件位置：`$XDG_STATE_HOME/niri-dock/dock.log`，退回 `~/.local/state/...`
pub(crate) fn log_file() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("niri-dock").join("dock.log")
}

pub(crate) fn stamp() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|t| t.format("%H:%M:%S%.3f").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn sink() -> Option<&'static std::sync::Mutex<std::fs::File>> {
    static SINK: OnceLock<Option<std::sync::Mutex<std::fs::File>>> = OnceLock::new();
    SINK.get_or_init(|| {
        let path = log_file();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()
            .map(std::sync::Mutex::new)
    })
    .as_ref()
}

pub(crate) fn log(level: Level, tag: &str, msg: &str) {
    if level > self::level() {
        return;
    }
    let line = format!("{} {:<5} {:<8} {}", stamp(), level.label(), tag, msg);
    eprintln!("{line}");
    // 同时落盘：无论 dock 是被 systemd、niri 还是命令行拉起来的，日志都在
    if let Some(sink) = sink() {
        use std::io::Write;
        if let Ok(mut file) = sink.lock() {
            let _ = writeln!(file, "{line}");
        }
    }
}

#[macro_export]
macro_rules! log_error {
    ($tag:expr, $($arg:tt)*) => {
        $crate::log::log($crate::log::Level::Error, $tag, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($tag:expr, $($arg:tt)*) => {
        $crate::log::log($crate::log::Level::Warn, $tag, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_info {
    ($tag:expr, $($arg:tt)*) => {
        $crate::log::log($crate::log::Level::Info, $tag, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_debug {
    ($tag:expr, $($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Debug) {
            $crate::log::log($crate::log::Level::Debug, $tag, &format!($($arg)*))
        }
    };
}
