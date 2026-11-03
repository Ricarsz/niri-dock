//! 进程探测：判断某个桌面应用是否已经在本机跑着。
//!
//! 用途是「没有窗口 ≠ 没在运行」：QQ 登录后会缩回托盘，`niri msg windows`
//! 里查不到它，但进程还在，这时再启动一份只会弹「重复登录」。

use std::path::Path;

use crate::desktop::AppEntry;

/// 这个应用是否已经在本机跑着——哪怕它没有窗口。
///
/// 名字对不上很常见：QQ 的 Exec 是包装脚本 `linuxqq-wayland-fix`，真正在跑的
/// 是 `/opt/QQ/qq`。所以 exec 的第一个词和 StartupWMClass 都拿来比，只做
/// 不区分大小写的整词比较，避免 `qq` 这种短名字误伤别的进程。
pub(crate) fn app_running(entry: &AppEntry) -> bool {
    find_pid(entry).is_some()
}

/// 找到跑着这个应用的第一个进程号（没有就是 `None`）。
///
/// 托盘激活要拿 pid 去 D-Bus 上认领 StatusNotifierItem，所以这里把扫描单独拆出来。
pub(crate) fn find_pid(entry: &AppEntry) -> Option<i32> {
    let names = process_names(entry);
    if names.is_empty() {
        return None;
    }
    let procs = std::fs::read_dir("/proc").ok()?;
    for item in procs.flatten() {
        let file = item.file_name();
        let Some(pid) = file.to_str() else { continue };
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let dir = item.path();
        if process_name_of(&dir).iter().any(|n| names.contains(n)) {
            return pid.parse().ok();
        }
    }
    None
}

/// 用来和进程名比对的候选：exec 第一个词的文件名、StartupWMClass
fn process_names(entry: &AppEntry) -> Vec<String> {
    let mut names = Vec::new();
    let exec_first = entry.exec.split_whitespace().next().unwrap_or("");
    if let Some(base) = Path::new(exec_first).file_name().and_then(|s| s.to_str()) {
        names.push(base.to_lowercase());
    }
    if let Some(w) = &entry.wmclass {
        names.push(w.to_lowercase());
    }
    // 太短的名字（单个字符）容易误伤
    names.retain(|n| n.len() >= 2);
    names
}

/// 一个进程的 comm 与 argv[0] 的文件名（都转小写）
fn process_name_of(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(comm) = std::fs::read_to_string(dir.join("comm")) {
        let name = comm.trim();
        if !name.is_empty() {
            names.push(name.to_lowercase());
        }
    }
    if let Ok(raw) = std::fs::read(dir.join("cmdline")) {
        let first = raw.split(|b| *b == 0).next().unwrap_or(&[]);
        if let Ok(line) = std::str::from_utf8(first) {
            if let Some(base) = Path::new(line).file_name().and_then(|s| s.to_str()) {
                let base = base.to_lowercase();
                if !names.contains(&base) {
                    names.push(base);
                }
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, wmclass: Option<&str>, exec: &str) -> AppEntry {
        AppEntry {
            id: id.to_string(),
            name: id.to_string(),
            icon: "icon".into(),
            exec: exec.to_string(),
            wmclass: wmclass.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn process_names_cover_wrapper_and_wmclass() {
        // QQ 的 Exec 是包装脚本，真正在跑的是 /opt/QQ/qq（comm=qq）
        let names = process_names(&entry(
            "linuxqq-wayland-fix",
            Some("QQ"),
            "linuxqq-wayland-fix %U",
        ));
        assert!(names.contains(&"linuxqq-wayland-fix".to_string()));
        assert!(names.contains(&"qq".to_string()));
    }

    #[test]
    fn app_running_finds_the_current_process() {
        let comm = std::fs::read_to_string("/proc/self/comm").unwrap();
        let comm = comm.trim().to_string();
        assert!(!comm.is_empty());
        // exec 直接写成当前进程名，必然能匹配到自己
        assert!(app_running(&entry("self", None, comm.as_str())));
        assert!(!app_running(&entry(
            "nope",
            None,
            "definitely-not-running-xyz"
        )));
    }
}
