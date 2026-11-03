//! 配置目录变化 → 重建面板。
//!
//! 优先进程内的目录监视（inotify），拿不到监视器就退回 600ms 轮询。
//! 一次保存往往伴随好几个事件（写 `.tmp`、rename、属性变化），所以统一做 120ms 防抖：
//! 只留最后一次，面板不会因为一次保存重建好几遍。

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::config::config_mtime;
use crate::log_debug;
use crate::log_warn;

/// 盯着 `dir` 里的 `config.json*`，有变化就调用一次 `rebuild`
pub(crate) fn watch(dir: PathBuf, rebuild: Rc<dyn Fn()>) {
    let monitor = gtk::gio::File::for_path(&dir).monitor_directory(
        gtk::gio::FileMonitorFlags::WATCH_MOVES,
        gtk::gio::Cancellable::NONE,
    );

    match monitor {
        Ok(monitor) => {
            let pending: Rc<std::cell::RefCell<Option<glib::SourceId>>> =
                Rc::new(std::cell::RefCell::new(None));
            monitor.connect_changed(move |_, file, _, event| {
                let name = file
                    .basename()
                    .map(|b| b.to_string_lossy().to_string())
                    .unwrap_or_default();
                log_debug!("config", "监视事件 {event:?} {name}");
                // save_pins/save_config 是「写 .tmp 再 rename」，rename 只会在源文件上
                // 报 Renamed，所以 config.json.tmp 的动静也算配置变化
                if !name.starts_with("config.json") {
                    return;
                }
                use gtk::gio::FileMonitorEvent as E;
                if matches!(
                    event,
                    E::Deleted | E::MovedOut | E::Unmounted | E::PreUnmount
                ) {
                    return;
                }
                if let Some(id) = pending.borrow_mut().take() {
                    id.remove();
                }
                let rebuild = rebuild.clone();
                let pending2 = pending.clone();
                let id = glib::timeout_add_local_once(Duration::from_millis(120), move || {
                    *pending2.borrow_mut() = None;
                    rebuild();
                });
                *pending.borrow_mut() = Some(id);
            });
            // 监视器必须活着
            std::mem::forget(monitor);
        }
        Err(err) => {
            log_warn!("config", "无法监视配置目录，退回轮询: {err}");
            let last = Rc::new(std::cell::RefCell::new(config_mtime()));
            glib::timeout_add_local(Duration::from_millis(600), move || {
                let now = config_mtime();
                let changed = now != *last.borrow();
                if changed {
                    *last.borrow_mut() = now;
                    rebuild();
                }
                glib::ControlFlow::Continue
            });
        }
    }
}
