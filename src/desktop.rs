//! desktop 条目解析：从 `*.desktop` 里取名字、图标、命令行，并做窗口匹配。

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

// ─────────────────────── desktop 条目 ───────────────────────

#[derive(Debug, Clone, Default)]
pub(crate) struct AppEntry {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) exec: String,
    pub(crate) icon: String,
    pub(crate) wmclass: Option<String>,
}

pub(crate) fn data_dirs() -> Vec<PathBuf> {
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

pub(crate) fn parse_desktop(id: &str, path: &Path) -> Option<AppEntry> {
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

pub(crate) fn load_entries() -> HashMap<String, AppEntry> {
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

pub(crate) fn match_keys(entry: &AppEntry) -> Vec<String> {
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

pub(crate) fn app_matches(keys: &[String], app_id: &str) -> bool {
    let app = app_id.to_lowercase();
    if app.is_empty() {
        return false;
    }
    keys.iter().any(|k| !k.is_empty() && (app == *k || app.contains(k)))
}

pub(crate) fn strip_field_codes(exec: &str) -> String {
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

pub(crate) fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

pub(crate) fn spawn_exec(exec: &str) {
    let cmd = strip_field_codes(exec);
    if cmd.is_empty() {
        return;
    }
    // 关键：从 dock 继承来的 cwd 可能是任意目录（比如 dock 是从项目目录启动的），
    // 启动的应用应当落在用户家目录，而不是 dock 的工作目录。
    let home = home_dir();
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg(format!("setsid {cmd} >/dev/null 2>&1 &"));
    if let Some(home) = home {
        command.current_dir(home);
    }
    // dock 可能是从一个非交互环境拉起来的（构建脚本、系统服务、别的程序），
    // 那种环境里的 NO_COLOR / TERM=dumb / COLORTERM 会一路继承进终端和里面的命令，
    // 让 eza、bat 之类认为「不该上色」。这里统一清掉，具体终端（foot/kitty/ghostty…）
    // 会自己给子进程设 TERM/COLORTERM，不需要我们代劳。
    for var in ["NO_COLOR", "TERM", "COLORTERM", "CLICOLOR", "CLICOLOR_FORCE"] {
        command.env_remove(var);
    }
    let _ = command.spawn();
}

