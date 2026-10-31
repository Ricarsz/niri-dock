//! 配置：读写 `~/.config/niri-dock/config.json`，缺字段用默认值兜底。

use std::env;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ─────────────────────────── 配置 ───────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct Config {
    #[serde(default)]
    pub(crate) pins: Vec<String>,
    #[serde(default = "d_icon_size")]
    pub(crate) icon_size: i32,
    #[serde(default = "d_gap")]
    pub(crate) gap: i32,
    #[serde(default = "d_autohide")]
    pub(crate) autohide: bool,
    /// 鼠标停在图标上时是否显示应用名提示
    #[serde(default = "d_true")]
    pub(crate) tooltip: bool,
    /// 右键图标是否弹出应用菜单
    #[serde(default = "d_true")]
    pub(crate) context_menu: bool,
    /// 触发区高度：鼠标进入屏幕底边这条窄带才弹出 dock
    #[serde(default = "d_hot_zone")]
    pub(crate) hot_zone: i32,
    /// 弹出/收起动画时长（毫秒）
    #[serde(default = "d_reveal_ms")]
    pub(crate) reveal_ms: u64,
    #[serde(default = "d_delay")]
    pub(crate) hide_delay_ms: u64,
    /// 悬停放大（macOS 风，鼠标下的图标变大、邻居次之）
    #[serde(default = "d_true")]
    pub(crate) magnify: bool,
    /// 放大峰值（倍数）
    #[serde(default = "d_peak")]
    pub(crate) magnify_peak: f64,
    /// 图标之间的空隙（像素，加在图标宽度之外）
    #[serde(default = "d_spacing")]
    pub(crate) spacing: i32,
    /// 图标所在格子的边长：决定 dock 栏的长宽，图标在里面居中
    #[serde(default = "d_slot")]
    pub(crate) slot: i32,
    /// 面板内边距
    #[serde(default = "d_padding")]
    pub(crate) padding: i32,
    /// 停靠边：bottom / top / left / right
    #[serde(default = "d_position")]
    pub(crate) position: String,
    /// 主题：dark / light
    #[serde(default = "d_theme")]
    pub(crate) theme: String,
    /// 面板不透明度 0.0 - 1.0
    #[serde(default = "d_opacity")]
    pub(crate) panel_opacity: f64,
    /// 面板圆角
    #[serde(default = "d_radius")]
    pub(crate) panel_radius: i32,
}

pub(crate) fn d_spacing() -> i32 { 12 }
pub(crate) fn d_slot() -> i32 { 64 }
pub(crate) fn d_padding() -> i32 { 8 }
pub(crate) fn d_position() -> String { "bottom".into() }
pub(crate) fn d_theme() -> String { "dark".into() }
pub(crate) fn d_opacity() -> f64 { 0.92 }
pub(crate) fn d_radius() -> i32 { 22 }

pub(crate) fn d_true() -> bool { true }
pub(crate) fn d_hot_zone() -> i32 { 6 }
pub(crate) fn d_reveal_ms() -> u64 { 280 }
pub(crate) fn d_peak() -> f64 { 1.5 }

pub(crate) fn d_icon_size() -> i32 { 52 }
pub(crate) fn d_gap() -> i32 { 10 }
pub(crate) fn d_autohide() -> bool { true }
pub(crate) fn d_delay() -> u64 { 650 }

impl Default for Config {
    fn default() -> Self {
        Self {
            pins: vec![],
            icon_size: d_icon_size(),
            gap: d_gap(),
            autohide: d_autohide(),
            tooltip: d_true(),
            context_menu: d_true(),
            hot_zone: d_hot_zone(),
            reveal_ms: d_reveal_ms(),
            hide_delay_ms: d_delay(),
            magnify: d_true(),
            magnify_peak: d_peak(),
            spacing: d_spacing(),
            slot: d_slot(),
            padding: d_padding(),
            position: d_position(),
            theme: d_theme(),
            panel_opacity: d_opacity(),
            panel_radius: d_radius(),
        }
    }
}

pub(crate) fn config_dir() -> PathBuf {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var("HOME").unwrap_or_default()).join(".config"));
    base.join("niri-dock")
}

pub(crate) fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub(crate) fn save_config(cfg: &Config) {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = config_path();
    let tmp = path.with_extension("json.tmp");
    if let Ok(text) = serde_json::to_string_pretty(cfg) {
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

pub(crate) fn load_config() -> Config {
    let dir = config_dir();
    let path = dir.join("config.json");
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<Config>(&text) {
            return cfg;
        }
    }
    let cfg = Config {
        pins: default_pins(),
        ..Default::default()
    };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(text) = serde_json::to_string_pretty(&cfg) {
        let _ = std::fs::write(&path, text);
    }
    cfg
}

pub(crate) fn default_pins() -> Vec<String> {
    vec![
        "foot".into(),
        "org.gnome.Nautilus".into(),
        "google-chrome".into(),
        "firefox-developer-edition".into(),
        "com.shellyorg.shelly".into(),
        "flclash".into(),
        "deepseek-harness".into(),
        "ai.opencode.desktop".into(),
        "mpv-glass".into(),
    ]
}

pub(crate) fn config_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(config_path())
        .and_then(|m| m.modified())
        .ok()
}

