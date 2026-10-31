//! niri-dock —— 给 niri 用的 macOS 风 dock（GTK4 + gtk4-layer-shell + niri IPC）
//!
//! 结构：
//!   - 配置 ~/.config/niri-dock/config.json（pins / icon_size / 自动隐藏等）
//!   - niri IPC：一个长连接订阅事件流维护窗口表，另一个连接按需发 FocusWindow / CloseWindow
//!   - UI：layer-shell 底部居中的圆角面板，每个 pin 一个按钮，运行中显示小圆点

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gtk4 as gtk;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde::{Deserialize, Serialize};

mod settings;
use serde_json::Value;

// ─────────────────────────── 配置 ───────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Config {
    #[serde(default)]
    pins: Vec<String>,
    #[serde(default = "d_icon_size")]
    icon_size: i32,
    #[serde(default = "d_gap")]
    gap: i32,
    #[serde(default = "d_autohide")]
    autohide: bool,
    #[serde(default = "d_delay")]
    hide_delay_ms: u64,
    /// 悬停放大（macOS 风，鼠标下的图标变大、邻居次之）
    #[serde(default = "d_true")]
    magnify: bool,
    /// 放大峰值（倍数）
    #[serde(default = "d_peak")]
    magnify_peak: f64,
    /// 图标之间的空隙（像素，加在图标宽度之外）
    #[serde(default = "d_spacing")]
    spacing: i32,
    /// 图标所在格子的边长：决定 dock 栏的长宽，图标在里面居中
    #[serde(default = "d_slot")]
    slot: i32,
    /// 面板内边距
    #[serde(default = "d_padding")]
    padding: i32,
    /// 停靠边：bottom / top / left / right
    #[serde(default = "d_position")]
    position: String,
    /// 主题：dark / light
    #[serde(default = "d_theme")]
    theme: String,
    /// 面板不透明度 0.0 - 1.0
    #[serde(default = "d_opacity")]
    panel_opacity: f64,
    /// 面板圆角
    #[serde(default = "d_radius")]
    panel_radius: i32,
}

fn d_spacing() -> i32 { 12 }
fn d_slot() -> i32 { 64 }
fn d_padding() -> i32 { 8 }
fn d_position() -> String { "bottom".into() }
fn d_theme() -> String { "dark".into() }
fn d_opacity() -> f64 { 0.92 }
fn d_radius() -> i32 { 22 }

fn d_true() -> bool { true }
fn d_peak() -> f64 { 1.5 }

fn d_icon_size() -> i32 { 52 }
fn d_gap() -> i32 { 10 }
fn d_autohide() -> bool { true }
fn d_delay() -> u64 { 650 }

impl Default for Config {
    fn default() -> Self {
        Self {
            pins: vec![],
            icon_size: d_icon_size(),
            gap: d_gap(),
            autohide: d_autohide(),
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

fn config_dir() -> PathBuf {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var("HOME").unwrap_or_default()).join(".config"));
    base.join("niri-dock")
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

fn save_config(cfg: &Config) {
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

fn load_config() -> Config {
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

fn default_pins() -> Vec<String> {
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

// ─────────────────────── desktop 条目 ───────────────────────

#[derive(Debug, Clone, Default)]
struct AppEntry {
    id: String,
    name: String,
    exec: String,
    icon: String,
    wmclass: Option<String>,
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(x) = env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(x));
    } else if let Some(home) = env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share"));
    }
    for d in env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
        .split(':')
    {
        if !d.is_empty() {
            dirs.push(PathBuf::from(d));
        }
    }
    dirs
}

fn parse_desktop(id: &str, path: &Path) -> Option<AppEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut entry = AppEntry { id: id.to_string(), ..Default::default() };
    let mut in_main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main || line.starts_with('#') {
            continue;
        }
        let (k, v) = match line.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        match k {
            "Name" if entry.name.is_empty() => entry.name = v.to_string(),
            "Exec" => entry.exec = v.to_string(),
            "Icon" => entry.icon = v.to_string(),
            "StartupWMClass" => entry.wmclass = Some(v.to_string()),
            _ => {}
        }
    }
    if entry.exec.is_empty() {
        return None;
    }
    if entry.name.is_empty() {
        entry.name = id.to_string();
    }
    Some(entry)
}

fn load_entries() -> HashMap<String, AppEntry> {
    let mut map = HashMap::new();
    for base in data_dirs() {
        let appdir = base.join("applications");
        let Ok(rd) = std::fs::read_dir(&appdir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            if map.contains_key(stem) {
                continue;
            }
            if let Some(entry) = parse_desktop(stem, &path) {
                map.insert(stem.to_string(), entry);
            }
        }
    }
    map
}

fn match_keys(entry: &AppEntry) -> Vec<String> {
    let mut keys = vec![entry.id.to_lowercase()];
    if let Some(w) = &entry.wmclass {
        keys.push(w.to_lowercase());
    }
    let exec_first = entry.exec.split_whitespace().next().unwrap_or("");
    let base = Path::new(exec_first).file_name().and_then(|s| s.to_str()).unwrap_or("");
    if !base.is_empty() {
        keys.push(base.to_lowercase());
    }
    keys.dedup();
    keys
}

fn app_matches(keys: &[String], app_id: &str) -> bool {
    let app = app_id.to_lowercase();
    if app.is_empty() {
        return false;
    }
    keys.iter().any(|k| !k.is_empty() && (app == *k || app.contains(k)))
}

fn strip_field_codes(exec: &str) -> String {
    let mut out = String::new();
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            if let Some(n) = chars.next() {
                if n == '%' {
                    out.push('%');
                }
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

fn spawn_exec(exec: &str) {
    let cmd = strip_field_codes(exec);
    if cmd.is_empty() {
        return;
    }
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("setsid {cmd} >/dev/null 2>&1 &"))
        .spawn();
}

// ───────────────────────── niri IPC ─────────────────────────

#[derive(Debug, Clone, Default)]
struct Win {
    id: i64,
    title: String,
    app_id: String,
    is_focused: bool,
}

fn socket_path() -> Option<PathBuf> {
    env::var_os("NIRI_SOCKET").map(PathBuf::from)
}

fn niri_request(req: Value) -> Option<Value> {
    let path = socket_path()?;
    let stream = UnixStream::connect(path).ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut writer = stream;
    let mut line = serde_json::to_string(&req).ok()?;
    line.push('\n');
    writer.write_all(line.as_bytes()).ok()?;
    let mut resp = String::new();
    reader.read_line(&mut resp).ok()?;
    serde_json::from_str(&resp).ok()
}

fn niri_focus(id: i64) {
    let _ = niri_request(serde_json::json!({ "FocusWindow": { "id": id } }));
}

fn niri_close(id: i64) {
    let _ = niri_request(serde_json::json!({ "CloseWindow": { "id": id } }));
}

fn parse_win(v: &Value) -> Option<Win> {
    Some(Win {
        id: v.get("id")?.as_i64()?,
        title: v.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        app_id: v.get("app_id").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        is_focused: v.get("is_focused").and_then(|t| t.as_bool()).unwrap_or(false),
    })
}

fn fetch_all(windows: &Arc<Mutex<HashMap<i64, Win>>>) {
    let Some(resp) = niri_request(serde_json::json!("Windows")) else { return };
    let Some(list) = resp
        .get("Ok")
        .and_then(|v| v.get("Windows"))
        .and_then(|v| v.as_array())
    else {
        return;
    };
    let mut map = windows.lock().unwrap();
    map.clear();
    for v in list {
        if let Some(w) = parse_win(v) {
            map.insert(w.id, w);
        }
    }
}

fn spawn_event_thread(windows: Arc<Mutex<HashMap<i64, Win>>>, epoch: Arc<AtomicU64>) {
    std::thread::spawn(move || loop {
        let Some(path) = socket_path() else {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        let Ok(stream) = UnixStream::connect(&path) else {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        let mut writer = match stream.try_clone() {
            Ok(w) => w,
            Err(_) => {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        if writer.write_all(b"\"EventStream\"\n").is_err() {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        let reader = BufReader::new(stream);
        let mut lines = reader.lines();
        let _ = lines.next(); // EventStream 的应答
        fetch_all(&windows);
        epoch.fetch_add(1, Ordering::SeqCst);

        for line in lines {
            let Ok(text) = line else { break };
            let Ok(ev) = serde_json::from_str::<Value>(&text) else { continue };
            let mut changed = false;
            if let Some(v) = ev.get("WindowOpenedOrChanged").and_then(|v| v.get("window")) {
                if let Some(w) = parse_win(v) {
                    windows.lock().unwrap().insert(w.id, w);
                    changed = true;
                }
            }
            if let Some(v) = ev.get("WindowClosed").and_then(|v| v.get("id")).and_then(|v| v.as_i64()) {
                windows.lock().unwrap().remove(&v);
                changed = true;
            }
            if let Some(list) = ev
                .get("WindowsChanged")
                .and_then(|v| v.get("windows"))
                .and_then(|v| v.as_array())
            {
                let mut map = windows.lock().unwrap();
                map.clear();
                for v in list {
                    if let Some(w) = parse_win(v) {
                        map.insert(w.id, w);
                    }
                }
                changed = true;
            }
            if let Some(focused) = ev.get("WindowFocusChanged").and_then(|v| v.get("id")) {
                let focused = focused.as_i64();
                let mut map = windows.lock().unwrap();
                for w in map.values_mut() {
                    w.is_focused = Some(w.id) == focused;
                }
                changed = true;
            }
            if changed {
                epoch.fetch_add(1, Ordering::SeqCst);
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    });
}

// ─────────────────────────── UI ───────────────────────────

/// 放大档位：1.0 → 1.30，每档 0.02
const MAG_STEPS: usize = 15;
const MAG_STEP: f64 = 0.02;

/// 悬停放大：目标倍数按距离做钟形衰减
fn magnify_target(cfg: &Config, index: usize, hover: Option<usize>) -> f64 {
    if !cfg.magnify {
        return 1.0;
    }
    match hover {
        Some(h) => {
            let d = (index as f64 - h as f64).abs();
            let peak = cfg.magnify_peak.clamp(1.0, 1.30);
            1.0 + (peak - 1.0) * (-(d * d) / 0.9).exp()
        }
        None => 1.0,
    }
}

fn css_text(cfg: &Config) -> String {
    let light = cfg.theme.eq_ignore_ascii_case("light");
    let opacity = cfg.panel_opacity.clamp(0.0, 1.0);
    let (bg_r, bg_g, bg_b) = if light { (246.0, 246.0, 248.0) } else { (38.0, 38.0, 42.0) };
    // 圆角跟着图标尺寸走，小图标别留一个巨圆角
    let radius = cfg.panel_radius.max(0);
    let pad = cfg.padding.max(0);
    let (border, dot, _shadow, hover) = if light {
        (
            "rgba(0, 0, 0, 0.12)",
            "rgba(0, 0, 0, 0.55)",
            "rgba(0, 0, 0, 0.22)",
            "rgba(0, 0, 0, 0.08)",
        )
    } else {
        (
            "rgba(255, 255, 255, 0.13)",
            "rgba(255, 255, 255, 0.9)",
            "rgba(0, 0, 0, 0.45)",
            "rgba(255, 255, 255, 0.14)",
        )
    };

    let mut s = String::new();
    s.push_str(&format!(
        r#"
window, window.background {{
    background-color: transparent;
    background-image: none;
    box-shadow: none;
    border: none;
    outline: none;
}}

.dock-panel {{
    padding: {pad}px {pad}px;
    border-radius: {radius}px;
    border: 1px solid {border};
    background-color: rgba({bg_r}, {bg_g}, {bg_b}, {opacity});
    background-image: none;
}}

.dock-item {{
    padding: 0;
    margin: 0;
    border-radius: 14px;
    border: none;
    background-color: transparent;
    background-image: none;
    box-shadow: none;
    transition: transform 130ms cubic-bezier(0.25, 0.7, 0.3, 1);
    transform-origin: center bottom;
}}

.dock-item:active {{
    background-color: {hover};
}}

.dot {{
    min-width: 4px;
    min-height: 4px;
    border-radius: 999px;
    background-color: {dot};
    transition: opacity 160ms ease;
}}

.dot.hidden {{
    opacity: 0;
}}

.dock-menu {{
    padding: 6px;
}}

.dock-menu .menu-title {{
    font-weight: 700;
    margin: 4px 10px 6px 10px;
}}

.dock-menu button {{
    padding: 6px 12px;
    border-radius: 10px;
    background-color: transparent;
    border: none;
}}
"#
    ));

    s.push_str(".dock-strip { background-color: rgba(0, 0, 0, 0.004); }\n");
    // 放大档位：只改 transform（不进布局），过渡由上面的 CSS transition 负责
    for i in 1..=MAG_STEPS {
        let scale = 1.0 + MAG_STEP * i as f64;
        s.push_str(&format!(
            ".dock-item.m{i} {{ transform: scale({scale:.3}); }}\n"
        ));
    }
    s
}

struct Item {
    entry: AppEntry,
    button: gtk::Button,
    image: gtk::Image,
    dot: gtk::Box,
    level: std::cell::Cell<usize>,
}

struct Ui {
    cfg: Config,
    entries: Rc<HashMap<String, AppEntry>>,
    windows: Arc<Mutex<HashMap<i64, Win>>>,
    epoch: Arc<AtomicU64>,
    items: RefCell<Vec<Item>>,
    hover: std::cell::Cell<Option<usize>>,
    revealer: gtk::Revealer,
    panel: gtk::Box,
    window: gtk::ApplicationWindow,
    shown: Rc<std::cell::Cell<bool>>,
    hide_at: RefCell<Option<Instant>>,
}

impl Ui {
    fn window_for(&self, entry: &AppEntry) -> Option<i64> {
        let keys = match_keys(entry);
        let map = self.windows.lock().unwrap();
        let mut best: Option<&Win> = None;
        for w in map.values() {
            if !app_matches(&keys, &w.app_id) {
                continue;
            }
            best = match best {
                None => Some(w),
                Some(b) => {
                    if w.is_focused && !b.is_focused {
                        Some(w)
                    } else {
                        Some(b)
                    }
                }
            };
        }
        best.map(|w| w.id)
    }

    fn refresh_dots(&self) {
        for item in self.items.borrow().iter() {
            let running = self.window_for(&item.entry).is_some();
            if running {
                item.dot.remove_css_class("hidden");
            } else {
                item.dot.add_css_class("hidden");
            }
        }
    }

    fn show(&self) {
        if self.shown.get() {
            return;
        }
        self.revealer.set_visible(true);
        self.revealer.set_reveal_child(true);
        self.shown.set(true);
        *self.hide_at.borrow_mut() = None;
    }

    fn hide(&self) {
        if !self.shown.get() {
            return;
        }
        self.revealer.set_reveal_child(false);
        self.shown.set(false);
        // 收完之后把 revealer 整个藏掉：它自己会留 1px，那正是屏幕底那条细线
        let revealer = self.revealer.clone();
        let window = self.window.clone();
        let shown = self.shown.clone();
        glib::timeout_add_local_once(Duration::from_millis(260), move || {
            if !shown.get() {
                revealer.set_visible(false);
                // 窗口缩小后 GTK 只会重画脏区，最上面那 1px 会留着上一帧的面板边框
                window.queue_draw();
            }
        });
    }

    fn schedule_hide(&self) {
        if !self.cfg.autohide {
            return;
        }
        *self.hide_at.borrow_mut() =
            Some(Instant::now() + Duration::from_millis(self.cfg.hide_delay_ms));
    }

    /// 悬停变化时只切 CSS 类：真正插值由 GTK 的 CSS transition 在合成层上跑，
    /// 不需要每帧回调，也不改布局（槽位尺寸固定）。
    fn apply_magnification(&self) {
        let hover = self.hover.get();
        for (i, item) in self.items.borrow().iter().enumerate() {
            let scale = magnify_target(&self.cfg, i, hover);
            let level = (((scale - 1.0) / MAG_STEP).round() as usize).min(MAG_STEPS);
            if item.level.get() != level {
                if item.level.get() > 0 {
                    item.button.remove_css_class(&format!("m{}", item.level.get()));
                }
                if level > 0 {
                    item.button.add_css_class(&format!("m{level}"));
                }
                item.level.set(level);
            }
        }
    }

    /// 启动弹跳：几个离散帧位，靠 timeout 触发
    fn bounce(&self, idx: usize) {
        let image = match self.items.borrow().get(idx) {
            Some(item) => item.image.clone(),
            None => return,
        };
        for (delay, lift) in [(60u64, 20i32), (170, 4), (290, 13), (410, 2), (540, 0)] {
            let img = image.clone();
            glib::timeout_add_local_once(Duration::from_millis(delay), move || {
                img.set_margin_bottom(8 + lift);
            });
        }
    }
}

fn dump_matches() {
    let cfg = load_config();
    let entries = load_entries();
    let holder: Arc<Mutex<HashMap<i64, Win>>> = Arc::new(Mutex::new(HashMap::new()));
    fetch_all(&holder);
    let map: HashMap<i64, Win> = holder.lock().unwrap().clone();
    for pin in &cfg.pins {
        match entries.get(pin) {
            Some(e) => {
                let keys = match_keys(e);
                let hits: Vec<String> = map
                    .values()
                    .filter(|w| app_matches(&keys, &w.app_id))
                    .map(|w| w.app_id.clone())
                    .collect();
                println!("{pin:<26} icon={:<24} keys={:?} hits={:?}", e.icon, keys, hits);
            }
            None => println!("{pin:<26} <no desktop entry>"),
        }
    }
    for w in map.values() {
        println!("window {} app_id={} focused={}", w.id, w.app_id, w.is_focused);
    }
}

thread_local! {
    static DOCK_CSS: gtk::CssProvider = gtk::CssProvider::new();
}

fn apply_css(cfg: &Config) {
    let css = css_text(cfg);
    DOCK_CSS.with(|provider| {
        provider.load_from_string(&css);
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                provider,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
        }
    });
}

fn config_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(config_path())
        .and_then(|m| m.modified())
        .ok()
}

fn main() -> glib::ExitCode {
    if env::args().any(|a| a == "--settings") {
        return settings::run();
    }

    if env::args().any(|a| a == "--dump") {
        dump_matches();
        return glib::ExitCode::SUCCESS;
    }

    let app = gtk::Application::builder()
        .application_id("com.ricarse.niridock")
        .build();

    app.connect_startup(|_| {
        apply_css(&load_config());
    });

    app.connect_activate(move |app| {
        let entries = Rc::new(load_entries());
        let windows: Arc<Mutex<HashMap<i64, Win>>> = Arc::new(Mutex::new(HashMap::new()));
        let epoch = Arc::new(AtomicU64::new(0));
        spawn_event_thread(windows.clone(), epoch.clone());

        let mut cfg = load_config();
        apply_css(&cfg);
        cfg = load_config();
        let win = build_ui(app, cfg, entries.clone(), windows.clone(), epoch.clone());
        let slot: Rc<RefCell<Option<gtk::ApplicationWindow>>> = Rc::new(RefCell::new(Some(win)));
        let last = Rc::new(RefCell::new(config_mtime()));

        let app2 = app.clone();
        let slot2 = slot.clone();
        glib::timeout_add_local(Duration::from_millis(600), move || {
            let now = config_mtime();
            if now != *last.borrow() {
                *last.borrow_mut() = now;
                let cfg = load_config();
                apply_css(&cfg);
                let fresh = build_ui(&app2, cfg, entries.clone(), windows.clone(), epoch.clone());
                let old = slot2.borrow_mut().replace(fresh);
                if let Some(o) = old {
                    o.close();
                }
            }
            glib::ControlFlow::Continue
        });
    });

    app.run()
}

/// 图标在格子里距底边的距离：让图标大致居中，同时给底部运行圆点留位置
fn item_bottom_margin(slot: i32, icon: i32) -> i32 {
    ((slot - icon) / 2).clamp(0, 24) + 2
}

fn build_ui(
    app: &gtk::Application,
    cfg: Config,
    entries: Rc<HashMap<String, AppEntry>>,
    windows: Arc<Mutex<HashMap<i64, Win>>>,
    epoch: Arc<AtomicU64>,
) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .decorated(false)
        .resizable(false)
        .build();

    let position = cfg.position.to_ascii_lowercase();
    let horizontal = !matches!(position.as_str(), "left" | "right");

    window.init_layer_shell();
    window.set_namespace(Some("niri-dock"));
    window.set_layer(Layer::Top);
    let edge = match position.as_str() {
        "top" => Edge::Top,
        "left" => Edge::Left,
        "right" => Edge::Right,
        _ => Edge::Bottom,
    };
    window.set_anchor(edge, true);
    window.set_margin(edge, 0);
    window.set_exclusive_zone(0);
    window.set_keyboard_mode(KeyboardMode::None);

    let panel = gtk::Box::new(
        if horizontal { gtk::Orientation::Horizontal } else { gtk::Orientation::Vertical },
        0,
    );
    panel.add_css_class("dock-panel");
    match edge {
        Edge::Top => panel.set_margin_top(cfg.gap.max(0)),
        Edge::Left => panel.set_margin_start(cfg.gap.max(0)),
        Edge::Right => panel.set_margin_end(cfg.gap.max(0)),
        _ => panel.set_margin_bottom(cfg.gap.max(0)),
    }

    let transition = match position.as_str() {
        "top" => gtk::RevealerTransitionType::SlideDown,
        "left" => gtk::RevealerTransitionType::SlideRight,
        "right" => gtk::RevealerTransitionType::SlideLeft,
        _ => gtk::RevealerTransitionType::SlideUp,
    };
    let revealer = gtk::Revealer::builder()
        .transition_type(transition)
        .transition_duration(220)
        .reveal_child(true)
        .build();

    let ui = Rc::new(Ui {
        cfg: cfg.clone(),
        entries: entries.clone(),
        windows: windows.clone(),
        epoch: epoch.clone(),
        items: RefCell::new(Vec::new()),
        hover: std::cell::Cell::new(None),
        revealer: revealer.clone(),
        panel: panel.clone(),
        window: window.clone(),
        shown: Rc::new(std::cell::Cell::new(true)),
        hide_at: RefCell::new(None),
    });

    let mut buttons: Vec<gtk::Button> = Vec::new();
    // 放大是通过 transform 做的，不占布局；槽位只需要容纳常态图标 + 用户设定的间距
    // 槽位决定面板长宽，图标边长只影响图标本身 —— 所以改图标大小不会撑大 dock
    let across = cfg.slot.clamp(16, 160);
    let along = across + cfg.spacing.max(0);
    let (slot_w, slot_h) = if horizontal { (along, across) } else { (across, along) };

    for (i, pin) in cfg.pins.iter().enumerate() {
        let entry = entries.get(pin).cloned().unwrap_or_else(|| AppEntry {
            id: pin.clone(),
            name: pin.clone(),
            icon: "application-x-executable".to_string(),
            ..Default::default()
        });

        let icon_name = if entry.icon.is_empty() {
            "application-x-executable".to_string()
        } else {
            entry.icon.clone()
        };
        let image = gtk::Image::from_icon_name(&icon_name);
        image.set_pixel_size(cfg.icon_size.clamp(8, 256));
        image.set_overflow(gtk::Overflow::Visible);
        image.set_valign(gtk::Align::End);
        image.set_margin_bottom(item_bottom_margin(across, cfg.icon_size));

        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("dot");
        dot.add_css_class("hidden");
        dot.set_halign(gtk::Align::Center);
        dot.set_valign(gtk::Align::End);
        dot.set_margin_bottom(1);

        let overlay = gtk::Overlay::new();
        overlay.set_size_request(slot_w, slot_h);
        overlay.set_overflow(gtk::Overflow::Visible);
        overlay.set_child(Some(&image));
        overlay.add_overlay(&dot);

        let button = gtk::Button::builder()
            .has_frame(false)
            .tooltip_text(if entry.name.is_empty() { pin.as_str() } else { entry.name.as_str() })
            .child(&overlay)
            .build();
        button.add_css_class("dock-item");

        let motion = gtk::EventControllerMotion::new();
        {
            let u = ui.clone();
            motion.connect_enter(move |_, _, _| {
                if env::var("NIRI_DOCK_HOVER").is_ok() {
                    return;
                }
                u.show();
                *u.hide_at.borrow_mut() = None;
                if u.hover.get() != Some(i) {
                    u.hover.set(Some(i));
                    u.apply_magnification();
                }
            });
        }
        {
            let u = ui.clone();
            motion.connect_leave(move |_| {
                if env::var("NIRI_DOCK_HOVER").is_ok() {
                    return;
                }
                if u.hover.get() == Some(i) {
                    u.hover.set(None);
                    u.apply_magnification();
                }
            });
        }
        button.add_controller(motion);

        {
            let u = ui.clone();
            let e = entry.clone();
            button.connect_clicked(move |_| {
                let keys = match_keys(&e);
                let mut ids: Vec<(i64, bool)> = {
                    let map = u.windows.lock().unwrap();
                    map.values()
                        .filter(|w| app_matches(&keys, &w.app_id))
                        .map(|w| (w.id, w.is_focused))
                        .collect()
                };
                ids.sort_by_key(|(id, _)| *id);
                if ids.is_empty() {
                    spawn_exec(&e.exec);
                    u.bounce(i);
                    return;
                }
                let next = ids
                    .iter()
                    .position(|(_, focused)| *focused)
                    .map(|p| ids[(p + 1) % ids.len()].0)
                    .unwrap_or(ids[0].0);
                niri_focus(next);
            });
        }

        let pop = gtk::Popover::new();
        pop.add_css_class("dock-menu");
        {
            let bx = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let title = gtk::Label::new(Some(if entry.name.is_empty() {
                pin.as_str()
            } else {
                entry.name.as_str()
            }));
            title.add_css_class("menu-title");
            title.set_halign(gtk::Align::Start);
            bx.append(&title);

            let b_new = gtk::Button::with_label("新窗口");
            b_new.set_has_frame(false);
            {
                let e = entry.clone();
                let p = pop.clone();
                b_new.connect_clicked(move |_| {
                    spawn_exec(&e.exec);
                    p.popdown();
                });
            }
            bx.append(&b_new);

            let b_quit = gtk::Button::with_label("退出");
            b_quit.set_has_frame(false);
            {
                let u = ui.clone();
                let e = entry.clone();
                let p = pop.clone();
                b_quit.connect_clicked(move |_| {
                    let keys = match_keys(&e);
                    let ids: Vec<i64> = {
                        let map = u.windows.lock().unwrap();
                        map.values()
                            .filter(|w| app_matches(&keys, &w.app_id))
                            .map(|w| w.id)
                            .collect()
                    };
                    for id in ids {
                        niri_close(id);
                    }
                    p.popdown();
                });
            }
            bx.append(&b_quit);
            pop.set_child(Some(&bx));
            pop.set_parent(&button);
        }
        {
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            let p = pop.clone();
            gesture.connect_released(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                p.popup();
            });
            button.add_controller(gesture);
        }

        buttons.push(button.clone());
        panel.append(&button);
        ui.items.borrow_mut().push(Item {
            entry,
            button: button.clone(),
            image,
            dot,
            level: std::cell::Cell::new(0),
        });
    }

    // 放大是 CSS transform，会超出槽位；layer surface 和 Revealer 都会按尺寸裁切，
    // 所以在面板四周留出「放大余量」，图标再大也不会被面板边吞掉。
    // 面板/Revealer/图层表面都会按自身尺寸裁切，放大超出的部分必须先留出空间，
    // 否则图标边缘会被 dock 面板「吞掉」。留多少按实际溢出量算，避免留一条无用的透明挡板。
    // 面板 / Revealer / 图层表面都会按自己的尺寸裁切，放大超出的部分要预先留出空间，
    // 否则图标会被 dock 面板「吞掉」一条边。
    // GTK 的 transform-origin 不生效，缩放以按钮中心为原点，所以向上最多长 格子 × (峰值-1)。
    let peak = cfg.magnify_peak.clamp(1.0, 1.30);
    let growth = if cfg.magnify {
        (across as f64 * (peak - 1.0)).ceil() as i32 + 8
    } else {
        0
    };
    // 图标比格子还大时也要留出超出的部分
    let tall_icon = (cfg.icon_size + item_bottom_margin(across, cfg.icon_size)
        - (across + cfg.padding.max(0)))
        .max(0);
    let room = growth.max(tall_icon);

    let pad_cross = gtk::Box::new(
        if horizontal { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal },
        0,
    );
    if horizontal {
        pad_cross.set_size_request(-1, room);
    } else {
        pad_cross.set_size_request(room, -1);
    }

    let line = gtk::Box::new(
        if horizontal { gtk::Orientation::Horizontal } else { gtk::Orientation::Vertical },
        0,
    );
    let pad_a = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pad_a.set_size_request(room, -1);
    let pad_b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pad_b.set_size_request(room, -1);
    line.append(&pad_a);
    line.append(&panel);
    line.append(&pad_b);

    let wrap = gtk::Box::new(
        if horizontal { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal },
        0,
    );
    // 让出面板朝屏幕内侧的生长方向
    if matches!(position.as_str(), "top" | "left") {
        wrap.append(&line);
        wrap.append(&pad_cross);
    } else {
        wrap.append(&pad_cross);
        wrap.append(&line);
    }

    revealer.set_child(Some(&wrap));

    let strip = gtk::Box::new(
        if horizontal { gtk::Orientation::Horizontal } else { gtk::Orientation::Vertical },
        0,
    );
    strip.set_size_request(if horizontal { -1 } else { 6 }, if horizontal { 6 } else { -1 });
    strip.add_css_class("dock-strip");

    let outer = gtk::Box::new(
        if horizontal { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal },
        0,
    );
    // 热区条必须贴在屏幕那条边上
    if matches!(position.as_str(), "top" | "left") {
        outer.append(&strip);
        outer.append(&revealer);
    } else {
        outer.append(&revealer);
        outer.append(&strip);
    }
    window.set_child(Some(&outer));

    if ui.cfg.autohide {
        let u = ui.clone();
        let motion = gtk::EventControllerMotion::new();
        motion.connect_enter(move |_, _, _| {
            u.show();
            *u.hide_at.borrow_mut() = None;
        });
        let u2 = ui.clone();
        motion.connect_leave(move |_| {
            if u2.hover.get().is_some() {
                u2.hover.set(None);
                u2.apply_magnification();
            }
            u2.schedule_hide();
        });
        window.add_controller(motion);

        let u3 = ui.clone();
        glib::timeout_add_local(Duration::from_millis(120), move || {
            let due = match *u3.hide_at.borrow() {
                Some(t) => Instant::now() >= t,
                None => false,
            };
            if due {
                u3.hide();
                *u3.hide_at.borrow_mut() = None;
            }
            glib::ControlFlow::Continue
        });

        if env::var("NIRI_DOCK_HOVER").is_err() {
            let u4 = ui.clone();
            glib::timeout_add_local_once(Duration::from_millis(2500), move || u4.hide());
        }
    }

    {
        let u = ui.clone();
        let last = std::cell::Cell::new(u64::MAX);
        glib::timeout_add_local(Duration::from_millis(120), move || {
            let e = u.epoch.load(Ordering::SeqCst);
            if last.get() != e {
                last.set(e);
                u.refresh_dots();
            }
            glib::ControlFlow::Continue
        });
    }

    if let Ok(v) = env::var("NIRI_DOCK_HOVER") {
        if let Ok(n) = v.parse::<usize>() {
            ui.hover.set(Some(n));
            ui.apply_magnification();
        }
    }
    if let Ok(v) = env::var("NIRI_DOCK_BOUNCE") {
        if let Ok(n) = v.parse::<usize>() {
            ui.bounce(n);
        }
    }
    if let Ok(v) = env::var("NIRI_DOCK_MENU") {
        if let Ok(n) = v.parse::<usize>() {
            if let Some(btn) = buttons.get(n).cloned() {
                glib::timeout_add_local_once(Duration::from_millis(700), move || {
                    btn.activate();
                });
            }
        }
    }

    ui.refresh_dots();
    window.present();
    window
}
