//! 构建 dock 面板。只有一个图层窗口：折叠时它缩成 hot_zone 大小的透明带，
//! 这条带本身就是热区，不再另开一个与主窗口重叠的热区 surface。
//!
//! 分工：`window` 管窗口/Revealer/透明底，`tile` 管单个图标与右键菜单，
//! `edit` 管编辑模式，纯几何在 `crate::dock::geometry`。

mod edit;
mod tile;
mod window;

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::config::{load_config, Config};
use crate::desktop::*;
use crate::log_debug;
use crate::niri::*;

use super::ui::Ui;

use edit::editing;
pub(crate) use edit::toggle_editing;

/// `niri-dock --dump`：把每个 pin 匹配到的窗口打出来，排查图标不亮/不聚焦用
pub(crate) fn dump_matches() {
    let cfg = load_config();
    let entries = load_entries();
    let holder: Arc<Mutex<HashMap<i64, Win>>> = Arc::new(Mutex::new(HashMap::new()));
    fetch_all(&holder);
    let map: HashMap<i64, Win> = holder.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for pin in &cfg.pins {
        match entries.get(pin) {
            Some(e) => {
                let keys = match_keys(e);
                let hits: Vec<String> = map
                    .values()
                    .filter(|w| app_matches(&keys, &w.app_id))
                    .map(|w| w.app_id.clone())
                    .collect();
                println!(
                    "{pin:<26} icon={:<24} keys={:?} hits={:?}",
                    e.icon, keys, hits
                );
            }
            None => println!("{pin:<26} <no desktop entry>"),
        }
    }
    for w in map.values() {
        println!(
            "window {} app_id={} focused={}",
            w.id, w.app_id, w.is_focused
        );
    }
}

pub(crate) fn build_ui(
    app: &gtk::Application,
    mut cfg: Config,
    entries: Rc<HashMap<String, AppEntry>>,
    windows: Arc<Mutex<HashMap<i64, Win>>>,
    epoch: Arc<AtomicU64>,
    reuse_window: Option<gtk::ApplicationWindow>,
    rebuild: &crate::cli::Rebuild,
) -> (gtk::ApplicationWindow, Rc<Ui>) {
    // 重建时复用已有窗口：只换内容，不销毁/重建 surface
    // （每建一次窗口要多花 ~1.4MB 显存/纹理，而且会闪一下）
    let reused = reuse_window.is_some();
    log_debug!(
        "build",
        "构建面板：{} 个图标，复用窗口={reused}",
        cfg.pins.len()
    );

    let editing_mode = editing();
    if editing_mode {
        // 编辑时别自动隐藏，不然手一停 dock 就滑走了
        cfg.autohide = false;
    }
    let position = cfg.position.to_ascii_lowercase();
    let horizontal = !matches!(position.as_str(), "left" | "right");

    let window = window::create(app, reuse_window);
    let shell = window::chrome(&window, &cfg, &position, horizontal);
    let (panel, revealer, slots) = (shell.panel.clone(), shell.revealer.clone(), shell.slots);

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
        hide_src: RefCell::new(None),
        window_motion: RefCell::new(None),
        menu_open: std::cell::Cell::new(0),
        popovers: RefCell::new(Vec::new()),
        menu_src: RefCell::new(None),
    });

    let mut buttons: Vec<gtk::Button> = Vec::new();
    let mut popovers: Vec<gtk::Popover> = Vec::new();

    for (i, pin) in cfg.pins.iter().enumerate() {
        let entry = entries.get(pin).cloned().unwrap_or_else(|| AppEntry {
            id: pin.clone(),
            name: pin.clone(),
            icon: "application-x-executable".to_string(),
            ..Default::default()
        });
        let (button, item, pop) = tile::build(&ui, &cfg, i, pin, entry, &slots, editing_mode);
        buttons.push(button.clone());
        panel.append(&button);
        ui.items.borrow_mut().push(item);
        if let Some(pop) = pop {
            ui.popovers.borrow_mut().push(pop.clone());
            popovers.push(pop);
        }
    }

    if ui.cfg.autohide {
        let u = ui.clone();
        let motion = gtk::EventControllerMotion::new();
        // 留一份给 Ui：定时隐藏到点时用它的 contains-pointer 复查指针位置
        *ui.window_motion.borrow_mut() = Some(motion.clone());
        motion.connect_enter(move |_, _, _| {
            u.show();
            u.cancel_hide();
        });
        let u2 = ui.clone();
        motion.connect_leave(move |_| {
            // 指针移进右键菜单时也会触发 leave：此时不要收起 dock，也不要让图标缩回去，
            // 否则菜单会跟着 dock 一起消失、看起来「点不中 + 卡顿」
            if u2.menu_open.get() > 0 {
                return;
            }
            if u2.hover.get().is_some() {
                u2.hover.set(None);
                u2.apply_magnification();
            }
            u2.schedule_hide();
        });
        window.add_controller(motion);

        if !reused && env::var("NIRI_DOCK_HOVER").is_err() {
            let u4 = ui.clone();
            glib::timeout_add_local_once(Duration::from_millis(2500), move || u4.hide());
        }
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
            if let Some(pop) = popovers.get(n).cloned() {
                glib::timeout_add_local_once(Duration::from_millis(700), move || {
                    pop.popup();
                });
            }
        }
    }

    ui.refresh_dots();
    window.present();

    // 一个应用都没有时给个入口，直接开设置窗口（添加应用在设置里做）
    if ui.cfg.pins.is_empty() {
        let tile = cfg.icon_size.clamp(8, 256) + 6;
        let empty = gtk::Button::with_label("设置");
        empty.add_css_class("dock-item");
        empty.add_css_class("dock-add");
        empty.set_size_request(tile * 2, tile);
        empty.set_tooltip_text(Some("dock 还是空的：在设置里添加应用"));
        empty.connect_clicked(|_| {
            let _ = std::process::Command::new("niri-dock")
                .arg("--settings")
                .spawn();
        });
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        holder.set_halign(gtk::Align::Center);
        holder.append(&empty);
        panel.append(&holder);
    }

    if editing_mode {
        panel.append(&edit::done_button(&cfg, slots.w, slots.h, rebuild));
    }

    // 调试：NIRI_DOCK_CLICK=<序号>，启动后直接点一下第 N 个图标。
    // 不用真的移鼠标，方便在没有指针注入的环境里验证点击行为。
    if let Some(index) = env::var("NIRI_DOCK_CLICK")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        if let Some(button) = buttons.get(index).cloned() {
            log_debug!("build", "调试点击：第 {index} 个图标");
            glib::timeout_add_local_once(Duration::from_millis(600), move || {
                button.emit_clicked();
            });
        }
    }

    (window, ui)
}
