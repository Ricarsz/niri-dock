//! 构建 dock 面板与横贯屏幕边的热区窗口。

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gtk4 as gtk;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::config::{load_config, Config};
use crate::desktop::*;
use crate::niri::*;

use super::ui::Ui;
use super::Item;

pub(crate) fn dump_matches() {
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

pub(crate) fn item_bottom_margin(slot: i32, icon: i32) -> i32 {
    ((slot - icon) / 2).clamp(0, 24) + 2
}

pub(crate) fn build_ui(
    app: &gtk::Application,
    cfg: Config,
    entries: Rc<HashMap<String, AppEntry>>,
    windows: Arc<Mutex<HashMap<i64, Win>>>,
    epoch: Arc<AtomicU64>,
) -> (gtk::ApplicationWindow, Option<gtk::ApplicationWindow>, Rc<Ui>) {
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
        .transition_duration(cfg.reveal_ms.clamp(60, 900) as u32)
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
        menu_open: std::cell::Cell::new(0),
        popovers: RefCell::new(Vec::new()),
        menu_close_at: RefCell::new(None),
    });

    let mut buttons: Vec<gtk::Button> = Vec::new();
    let mut popovers: Vec<gtk::Popover> = Vec::new();
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
        // 高亮画在这块比图标略大的底板上，而不是铺满整个槽位
        let glow = gtk::Box::new(gtk::Orientation::Vertical, 0);
        glow.add_css_class("dock-glow");
        let glow_pad = 6;
        glow.set_size_request(cfg.icon_size + glow_pad, cfg.icon_size + glow_pad);
        glow.set_halign(gtk::Align::Center);
        glow.set_valign(gtk::Align::End);
        glow.set_margin_bottom((item_bottom_margin(across, cfg.icon_size) - glow_pad / 2).max(0));
        image.set_valign(gtk::Align::Center);
        image.set_margin_bottom(0);
        glow.append(&image);
        overlay.set_child(Some(&glow));
        overlay.add_overlay(&dot);

        let button = gtk::Button::builder()
            .has_frame(false)
            .child(&overlay)
            .build();
        if cfg.tooltip {
            button.set_tooltip_text(Some(if entry.name.is_empty() {
                pin.as_str()
            } else {
                entry.name.as_str()
            }));
        }
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

        if !cfg.context_menu {
            buttons.push(button.clone());
            panel.append(&button);
            ui.items.borrow_mut().push(Item {
                entry,
                button,
                image,
                dot,
                slot: overlay.clone(),
                level: std::cell::Cell::new(0),
            });
            continue;
        }

        let pop = gtk::Popover::new();
        pop.add_css_class("dock-menu");
        // 自动关闭依赖 GTK 的键盘/指针 grab，而 layer surface 上 grab 不可靠，
        // 会出现「指针一进菜单就消失」，所以自己管生命周期
        pop.set_autohide(false);
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

            // 指针离开菜单 500ms 后自动收起（layer surface 上没法靠 GTK grab 的「点外面关闭」）
            {
                let u = ui.clone();
                let motion = gtk::EventControllerMotion::new();
                motion.connect_enter(move |_, _, _| {
                    *u.menu_close_at.borrow_mut() = None;
                });
                let u2 = ui.clone();
                motion.connect_leave(move |_| {
                    *u2.menu_close_at.borrow_mut() =
                        Some(Instant::now() + Duration::from_millis(500));
                });
                bx.add_controller(motion);
            }
            pop.set_child(Some(&bx));
            pop.set_parent(&button);
        }
        {
            let u = ui.clone();
            pop.connect_show(move |_| {
                u.menu_open.set(u.menu_open.get() + 1);
                *u.hide_at.borrow_mut() = None;
            });
            let u = ui.clone();
            pop.connect_closed(move |_| {
                u.menu_open.set(u.menu_open.get().saturating_sub(1));
                if u.menu_open.get() == 0 && u.hover.get().is_none() {
                    u.schedule_hide();
                }
            });
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
        popovers.push(pop.clone());
        ui.popovers.borrow_mut().push(pop.clone());
        panel.append(&button);
        ui.items.borrow_mut().push(Item {
            entry,
            button: button.clone(),
            image,
            dot,
            slot: overlay.clone(),
            level: std::cell::Cell::new(0),
        });
    }

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

    // 热区不在这个窗口里：它是一条横贯整个屏幕宽度的独立图层，
    // 这样鼠标碰到屏幕底边任何一个位置都能把 dock 唤出来。
    let outer = gtk::Box::new(
        if horizontal { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal },
        0,
    );
    outer.append(&revealer);
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

        let u3 = ui.clone();
        glib::timeout_add_local(Duration::from_millis(120), move || {
            let due = match *u3.hide_at.borrow() {
                Some(t) => Instant::now() >= t,
                None => false,
            };
            if due {
                if u3.menu_open.get() == 0 {
                    u3.hide();
                }
                *u3.hide_at.borrow_mut() = None;
            }
            let close = match *u3.menu_close_at.borrow() {
                Some(t) => Instant::now() >= t,
                None => false,
            };
            if close {
                *u3.menu_close_at.borrow_mut() = None;
                u3.close_menus();
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
            if let Some(pop) = popovers.get(n).cloned() {
                glib::timeout_add_local_once(Duration::from_millis(700), move || {
                    pop.popup();
                });
            }
        }
    }

    ui.refresh_dots();
    window.present();

    // 横贯整条屏幕边的热区：鼠标碰到屏幕底边任何位置都能唤出 dock
    let hot = if ui.cfg.autohide {
        Some(build_hot_window(app, &ui.cfg, &ui))
    } else {
        None
    };

    (window, hot, ui)
}

pub(crate) fn build_hot_window(
    app: &gtk::Application,
    cfg: &Config,
    ui: &Rc<Ui>,
) -> gtk::ApplicationWindow {
    let win = gtk::ApplicationWindow::builder()
        .application(app)
        .decorated(false)
        .resizable(false)
        .build();

    let position = cfg.position.to_ascii_lowercase();
    let horizontal = !matches!(position.as_str(), "left" | "right");
    let hot = cfg.hot_zone.clamp(1, 32);

    win.init_layer_shell();
    win.set_namespace(Some("niri-dock-hot"));
    win.set_layer(Layer::Top);
    // 沿屏幕那条边铺满整条边
    match position.as_str() {
        "top" => {
            win.set_anchor(Edge::Top, true);
            win.set_anchor(Edge::Left, true);
            win.set_anchor(Edge::Right, true);
        }
        "left" => {
            win.set_anchor(Edge::Left, true);
            win.set_anchor(Edge::Top, true);
            win.set_anchor(Edge::Bottom, true);
        }
        "right" => {
            win.set_anchor(Edge::Right, true);
            win.set_anchor(Edge::Top, true);
            win.set_anchor(Edge::Bottom, true);
        }
        _ => {
            win.set_anchor(Edge::Bottom, true);
            win.set_anchor(Edge::Left, true);
            win.set_anchor(Edge::Right, true);
        }
    }
    win.set_exclusive_zone(0);
    win.set_keyboard_mode(KeyboardMode::None);

    let strip = gtk::Box::new(
        if horizontal { gtk::Orientation::Horizontal } else { gtk::Orientation::Vertical },
        0,
    );
    strip.add_css_class("dock-strip");
    strip.set_size_request(
        if horizontal { -1 } else { hot },
        if horizontal { hot } else { -1 },
    );
    win.set_child(Some(&strip));

    let u = ui.clone();
    let motion = gtk::EventControllerMotion::new();
    motion.connect_enter(move |_, _, _| {
        u.show();
        *u.hide_at.borrow_mut() = None;
    });
    strip.add_controller(motion);

    win.present();
    win
}
