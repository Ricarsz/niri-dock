//! niri IPC：一条连接订阅事件流维护窗口表，另一条按需发 FocusWindow / CloseWindow。

use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use crate::{log_debug, log_info};
// ───────────────────────── niri IPC ─────────────────────────

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub(crate) struct Win {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) app_id: String,
    pub(crate) is_focused: bool,
}

pub(crate) fn socket_path() -> Option<PathBuf> {
    env::var_os("NIRI_SOCKET").map(PathBuf::from)
}

pub(crate) fn niri_request(req: Value) -> Option<Value> {
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

pub(crate) fn niri_focus(id: i64) {
    let _ = niri_request(serde_json::json!({ "FocusWindow": { "id": id } }));
}

pub(crate) fn niri_close(id: i64) {
    let _ = niri_request(serde_json::json!({ "CloseWindow": { "id": id } }));
}

pub(crate) fn parse_win(v: &Value) -> Option<Win> {
    Some(Win {
        id: v.get("id")?.as_i64()?,
        title: v.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        app_id: v.get("app_id").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        is_focused: v.get("is_focused").and_then(|t| t.as_bool()).unwrap_or(false),
    })
}

pub(crate) fn fetch_all(windows: &Arc<Mutex<HashMap<i64, Win>>>) {
    let Some(resp) = niri_request(serde_json::json!("Windows")) else { return };
    let Some(list) = resp
        .get("Ok")
        .and_then(|v| v.get("Windows"))
        .and_then(|v| v.as_array())
    else {
        return;
    };
    let mut map = windows.lock().unwrap_or_else(|e| e.into_inner());
    map.clear();
    for v in list {
        if let Some(w) = parse_win(v) {
            map.insert(w.id, w);
        }
    }
}

pub(crate) fn spawn_event_thread(
    windows: Arc<Mutex<HashMap<i64, Win>>>,
    epoch: Arc<AtomicU64>,
    tx: async_channel::Sender<()>,
) {
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
        log_info!("niri", "已连接 {}", path.display());
        let reader = BufReader::new(stream);
        let mut lines = reader.lines();
        let _ = lines.next(); // EventStream 的应答
        fetch_all(&windows);
        epoch.fetch_add(1, Ordering::SeqCst);
        let _ = tx.try_send(());

        for line in lines {
            let Ok(text) = line else {
                log_info!("niri", "事件流结束，准备重连");
                break;
            };
            let Ok(ev) = serde_json::from_str::<Value>(&text) else {
                log_debug!("niri", "跳过无法解析的事件: {text}");
                continue;
            };
            let mut changed = false;
            if let Some(v) = ev.get("WindowOpenedOrChanged").and_then(|v| v.get("window")) {
                if let Some(w) = parse_win(v) {
                    log_debug!("niri", "窗口变化 id={} app={}", w.id, w.app_id);
                    windows.lock().unwrap_or_else(|e| e.into_inner()).insert(w.id, w);
                    changed = true;
                }
            }
            if let Some(v) = ev.get("WindowClosed").and_then(|v| v.get("id")).and_then(|v| v.as_i64()) {
                log_debug!("niri", "窗口关闭 id={v}");
                windows.lock().unwrap_or_else(|e| e.into_inner()).remove(&v);
                changed = true;
            }
            if let Some(list) = ev
                .get("WindowsChanged")
                .and_then(|v| v.get("windows"))
                .and_then(|v| v.as_array())
            {
                log_debug!("niri", "窗口列表更新，共 {} 个", list.len());
                let mut map = windows.lock().unwrap_or_else(|e| e.into_inner());
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
                log_debug!("niri", "焦点切换 {focused:?}");
                let mut map = windows.lock().unwrap_or_else(|e| e.into_inner());
                for w in map.values_mut() {
                    w.is_focused = Some(w.id) == focused;
                }
                changed = true;
            }
            if changed {
                epoch.fetch_add(1, Ordering::SeqCst);
                // 事件驱动：窗口集合变化时通知 UI 立即刷新圆点，不再靠 120ms 轮询
                let _ = tx.try_send(());
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    });
}

