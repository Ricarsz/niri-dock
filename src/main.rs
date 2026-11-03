//! niri-dock —— 给 niri 用的 macOS 风 dock（GTK4 + gtk4-layer-shell + niri IPC）

mod cli;
mod config;
mod desktop;
mod dock;
mod icons;
mod log;
mod niri;
mod process;
mod settings;
mod style;

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtk4 as gtk;
use gtk::prelude::*;

use config::{config_mtime, load_config};
use desktop::load_entries;
use dock::build_ui;
use niri::{spawn_event_thread, Win};
use style::apply_css;
fn main() -> glib::ExitCode {
    cli::restore_sigpipe();

    let args: Vec<String> = env::args().skip(1).collect();
    let cmd = cli::parse(&args);
    if let Some(code) = cli::dispatch(cmd) {
        return code;
    }

    let app = gtk::Application::builder()
        .application_id(cli::APP_ID)
        .build();

    app.connect_startup(|_| {
        apply_css(&load_config());
    });

    app.connect_activate(move |app| {
        log_info!(
            "main",
            "niri-dock {} 启动 (pid {})，日志 {}",
            env!("CARGO_PKG_VERSION"),
            std::process::id(),
            log::log_file().display()
        );
        let entries = Rc::new(load_entries());
        let windows: Arc<Mutex<HashMap<i64, Win>>> = Arc::new(Mutex::new(HashMap::new()));
        let epoch = Arc::new(AtomicU64::new(0));
        // 事件驱动的圆点刷新通道：niri 事件线程 → GTK 主循环，空闲时零唤醒
        let (tx, rx) = async_channel::unbounded::<()>();
        spawn_event_thread(windows.clone(), epoch.clone(), tx.clone());

        let current: Rc<RefCell<Option<Rc<dock::Ui>>>> = Rc::new(RefCell::new(None));
        {
            let current = current.clone();
            glib::spawn_future_local(async move {
                while rx.recv().await.is_ok() {
                    if let Some(ui) = current.borrow().as_ref().cloned() {
                        ui.refresh_dots();
                    }
                }
            });
        }

        let mut cfg = load_config();
        log_info!(
            "config",
            "{} 个图标，位置 {}，主题 {}",
            cfg.pins.len(),
            cfg.position,
            cfg.theme
        );
        log_debug!("config", "配置目录 {}", config::config_dir().display());
        apply_css(&cfg);
        cfg = load_config();
        let trigger: cli::Rebuild = std::rc::Rc::new(std::cell::RefCell::new(None));
        let initial_pos = cfg.position.to_ascii_lowercase();
        let (win, ui) = build_ui(
            app,
            cfg,
            entries.clone(),
            windows.clone(),
            epoch.clone(),
            None,
            &trigger,
        );
        *current.borrow_mut() = Some(ui.clone());
        cli::register_actions(app, &ui, &trigger);
        cli::write_pid();
        let slot: Rc<RefCell<Option<gtk::ApplicationWindow>>> =
            Rc::new(RefCell::new(Some(win)));
        // 配置变化：用 inotify 的目录监视替代 600ms 轮询；事件再做一个 120ms 防抖。
        let rebuild = {
            let app2 = app.clone();
            let slot2 = slot.clone();
            let current2 = current.clone();
            let windows2 = windows.clone();
            let epoch2 = epoch.clone();
            let last_pos: Rc<RefCell<String>> =
                Rc::new(RefCell::new(initial_pos.clone()));
            let trigger2 = trigger.clone();
            Rc::new(move || {
                log_debug!("rebuild", "开始重建面板");
                let cfg = load_config();
                apply_css(&cfg);
                // 重新扫一遍 desktop，编辑模式里新加的应用才能立刻拿到名字和图标
                let entries = Rc::new(load_entries());
                // 位置没变就复用窗口（不重建 surface），变了才新建
                let same_pos = *last_pos.borrow() == cfg.position.to_ascii_lowercase();
                let old_win = slot2.borrow().clone();
                let was_shown = current2
                    .borrow()
                    .as_ref()
                    .map(|u| u.shown.get())
                    .unwrap_or(false);
                let reuse = if same_pos { old_win.clone() } else { None };
                *last_pos.borrow_mut() = cfg.position.to_ascii_lowercase();
                let (win, ui) = build_ui(
                    &app2,
                    cfg,
                    entries,
                    windows2.clone(),
                    epoch2.clone(),
                    reuse.clone(),
                    &trigger2,
                );
                if was_shown {
                    ui.show();
                }
                *current2.borrow_mut() = Some(ui.clone());
                cli::register_actions(&app2, &ui, &trigger2);
                *slot2.borrow_mut() = Some(win);
                if let Some(o) = old_win {
                    if Some(&o) != reuse.as_ref() {
                        o.close();
                    }
                }
            })
        };

        *trigger.borrow_mut() = Some(rebuild.clone());

        let dir = gtk::gio::File::for_path(config::config_dir());
        let monitor = dir.monitor_directory(gtk::gio::FileMonitorFlags::WATCH_MOVES, gtk::gio::Cancellable::NONE);
        match monitor {
            Ok(monitor) => {
                let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
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
                    if matches!(event, E::Deleted | E::MovedOut | E::Unmounted | E::PreUnmount) {
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
                let last = Rc::new(RefCell::new(config_mtime()));
                glib::timeout_add_local(Duration::from_millis(600), move || {
                    let now = config_mtime();
                    if now != *last.borrow() {
                        *last.borrow_mut() = now;
                        rebuild();
                    }
                    glib::ControlFlow::Continue
                });
            }
        }
    });

    app.connect_shutdown(|_| cli::clear_pid());

    app.run()
}

