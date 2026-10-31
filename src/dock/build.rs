//! 构建 dock 面板与横贯屏幕边的热区窗口。

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::config::{load_config, save_pins, Config};
use crate::desktop::*;
use crate::niri::*;

use super::ui::Ui;
use super::Item;

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

/// 编辑模式：拖动排序、点 × 删除、点 + 添加 .desktop
static EDITING: AtomicBool = AtomicBool::new(false);

pub(crate) fn editing() -> bool {
    EDITING.load(Ordering::Relaxed)
}

pub(crate) fn toggle_editing() -> bool {
    let now = !editing();
    EDITING.store(now, Ordering::Relaxed);
    now
}

/// 拖放落点：落在目标图标右半边就插到它后面（from 先被移除，索引要相应回退一格）
fn drop_index(from: usize, target: usize, right_half: bool) -> usize {
    let mut to = if right_half { target + 1 } else { target };
    if from < to {
        to -= 1;
    }
    to
}

fn reorder_pins(from: usize, to: usize) {
    let cfg = load_config();
    let mut pins = cfg.pins;
    if from >= pins.len() || to >= pins.len() || from == to {
        return;
    }
    let moved = pins.remove(from);
    pins.insert(to, moved);
    save_pins(&pins);
}

fn remove_pin(pin: &str) {
    let cfg = load_config();
    let pins: Vec<String> = cfg.pins.into_iter().filter(|p| p != pin).collect();
    save_pins(&pins);
}

/// 选中的 .desktop 不在标准目录里时，复制到 ~/.local/share/applications
fn adopt_desktop_file(path: &std::path::Path) -> Option<String> {
    let name = path.file_name()?.to_str()?.to_string();
    if !name.ends_with(".desktop") {
        return None;
    }
    let id = name.trim_end_matches(".desktop").to_string();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    let local = home.join(".local/share/applications");
    let _ = std::fs::create_dir_all(&local);
    let dest = local.join(&name);
    if path != dest {
        let _ = std::fs::copy(path, &dest);
        let _ = std::fs::set_permissions(
            &dest,
            std::os::unix::fs::PermissionsExt::from_mode(0o644),
        );
    }
    let mut cfg = load_config();
    if !cfg.pins.iter().any(|p| p == &id) {
        cfg.pins.push(id.clone());
        save_pins(&cfg.pins);
    }
    Some(id)
}

fn pick_desktop_file(parent: &gtk::Window) {
    let dialog = gtk::FileDialog::new();
    dialog.set_title("选择要添加的应用（.desktop）");
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("应用入口 (*.desktop)"));
    filter.add_pattern("*.desktop");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&filter));
    dialog.open(Some(parent), gtk::gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res {
            if let Some(path) = file.path() {
                adopt_desktop_file(&path);
            }
        }
    });
}

pub(crate) fn build_ui(
    app: &gtk::Application,
    mut cfg: Config,
    entries: Rc<HashMap<String, AppEntry>>,
    windows: Arc<Mutex<HashMap<i64, Win>>>,
    epoch: Arc<AtomicU64>,
    reuse_window: Option<gtk::ApplicationWindow>,
    rebuild: &crate::cli::Rebuild,
) -> (gtk::ApplicationWindow, Option<gtk::ApplicationWindow>, Rc<Ui>) {
    // 重建时复用已有窗口：只换内容，不销毁/重建 surface
    // （每建一次窗口要多花 ~1.4MB 显存/纹理，而且会闪一下）
    let reused = reuse_window.is_some();
    let window = match reuse_window {
        Some(w) => w,
        None => {
            let w = gtk::ApplicationWindow::builder()
                .application(app)
                .decorated(false)
                .resizable(false)
                .build();
            w.init_layer_shell();
            w.set_namespace(Some("niri-dock"));
            w.set_layer(Layer::Top);
            w.set_margin(Edge::Bottom, 0);
            w.set_exclusive_zone(0);
            w.set_keyboard_mode(KeyboardMode::None);
            w
        }
    };

    let editing_mode = editing();
    if editing_mode {
        // 编辑时别自动隐藏，不然手一停 dock 就滑走了
        cfg.autohide = false;
    }
    let position = cfg.position.to_ascii_lowercase();
    let horizontal = !matches!(position.as_str(), "left" | "right");

    let edge = match position.as_str() {
        "top" => Edge::Top,
        "left" => Edge::Left,
        "right" => Edge::Right,
        _ => Edge::Bottom,
    };
    window.set_anchor(edge, true);
    window.set_margin(edge, 0);

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
        hide_src: RefCell::new(None),
        menu_open: std::cell::Cell::new(0),
        popovers: RefCell::new(Vec::new()),
        menu_src: RefCell::new(None),
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

        if editing_mode {
            button.add_css_class("editing");

            let badge = gtk::Button::with_label("×");
            badge.add_css_class("dock-remove");
            badge.set_has_frame(false);
            badge.set_halign(gtk::Align::Start);
            badge.set_valign(gtk::Align::Start);
            badge.set_margin_start(1);
            badge.set_margin_top(1);
            {
                let pin_id = pin.clone();
                badge.connect_clicked(move |_| remove_pin(&pin_id));
            }
            overlay.add_overlay(&badge);

            // 拖动排序：拖自己，落到别的图标上就换位置
            let src = gtk::DragSource::new();
            src.set_actions(gtk::gdk::DragAction::MOVE);
            {
                let idx = i as u32;
                src.connect_prepare(move |_, _, _| {
                    Some(gtk::gdk::ContentProvider::for_value(&idx.to_value()))
                });
            }
            button.add_controller(src);

            let dst = gtk::DropTarget::new(u32::static_type(), gtk::gdk::DragAction::MOVE);
            {
                let target = i;
                let drop_btn = button.clone();
                dst.connect_drop(move |_, value, x, _| {
                    let Ok(from) = value.get::<u32>() else {
                        return false;
                    };
                    let half = drop_btn.width() as f64 / 2.0;
                    let to = drop_index(from as usize, target, x > half);
                    reorder_pins(from as usize, to);
                    true
                });
            }
            button.add_controller(dst);
        }

        let motion = gtk::EventControllerMotion::new();
        {
            let u = ui.clone();
            motion.connect_enter(move |_, _, _| {
                if env::var("NIRI_DOCK_HOVER").is_ok() {
                    return;
                }
                u.show();
                u.cancel_hide();
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
                    let map = u.windows.lock().unwrap_or_else(|e| e.into_inner());
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

        if !cfg.context_menu || editing_mode {
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
                        let map = u.windows.lock().unwrap_or_else(|e| e.into_inner());
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
                    u.cancel_menu_close();
                });
                let u2 = ui.clone();
                motion.connect_leave(move |_| {
                    u2.schedule_menu_close(500);
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
                u.cancel_hide();
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

    // 横贯整条屏幕边的热区：鼠标碰到屏幕底边任何位置都能唤出 dock
    if editing_mode || ui.cfg.pins.is_empty() {
        let add = gtk::Button::builder().has_frame(false).build();
        add.add_css_class("dock-item");
        add.add_css_class("dock-add");
        let tile = cfg.icon_size.clamp(8, 256) + 6;
        add.set_size_request(tile, tile);
        add.set_halign(gtk::Align::Center);
        add.set_valign(gtk::Align::Center);
        if !editing_mode {
            add.set_tooltip_text(Some("添加应用到 dock"));
        }
        let label = gtk::Label::new(Some("+"));
        let px = cfg.icon_size.clamp(8, 256) / 2;
        label.set_markup(&format!("<span font_size='{px}pt'>+</span>"));
        add.set_child(Some(&label));
        let overlay = gtk::Overlay::new();
        overlay.set_size_request(slot_w.max(24), slot_h.max(24));
        overlay.set_child(Some(&add));
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        holder.append(&overlay);
        {
            let win = window.clone();
            add.connect_clicked(move |_| pick_desktop_file(win.upcast_ref()));
        }
        // 放不进 panel 的话至少保证不被裁剪
        panel.append(&holder);
        buttons.push(add);
    }

    if editing_mode {
        let tile = cfg.icon_size.clamp(8, 256) + 6;
        let done = gtk::Button::builder().has_frame(false).build();
        done.add_css_class("dock-item");
        done.add_css_class("dock-add");
        done.set_size_request(tile, tile);
        done.set_halign(gtk::Align::Center);
        done.set_valign(gtk::Align::Center);
        let done_label = gtk::Label::new(Some("✓"));
        done_label.set_markup(&format!(
            "<span font_size='{}pt'>✓</span>",
            cfg.icon_size.clamp(8, 256) / 2
        ));
        done.set_child(Some(&done_label));
        let done_overlay = gtk::Overlay::new();
        done_overlay.set_size_request(slot_w.max(24), slot_h.max(24));
        done_overlay.set_child(Some(&done));
        let done_holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        done_holder.append(&done_overlay);
        {
            let r = rebuild.clone();
            done.connect_clicked(move |_| {
                crate::dock::toggle_editing();
                if let Some(f) = r.borrow().as_ref() {
                    f();
                }
            });
        }
        panel.append(&done_holder);
    }

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
        u.cancel_hide();
    });
    strip.add_controller(motion);

    win.present();
    win
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_index_moves_forward() {
        // 0 -> 2 的右半边：插到 2 之后，移除 0 后是 2
        assert_eq!(drop_index(0, 2, true), 2);
        // 0 -> 2 的左半边：插到 2 之前，移除 0 后是 1
        assert_eq!(drop_index(0, 2, false), 1);
    }

    #[test]
    fn drop_index_moves_backward() {
        assert_eq!(drop_index(3, 1, false), 1);
        assert_eq!(drop_index(3, 1, true), 2);
    }

    #[test]
    fn drop_index_stays_put() {
        assert_eq!(drop_index(1, 1, false), 1);
    }
}
