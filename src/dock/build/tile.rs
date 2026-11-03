//! 一个 dock 图标：图标、放大底板、运行圆点、点击行为、右键菜单。

use std::env;
use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::config::Config;
use crate::desktop::*;
use crate::niri::*;
use crate::process::app_running;

use super::super::geometry::{item_bottom_margin, Slots};
use super::super::launch;
use super::super::ui::Ui;
use super::super::{Action, Item};

/// 造一个图标按钮。`editing` 为真时额外挂拖放控制器（用来排序）。
/// 右键菜单开着时返回 Popover，交给调用方登记与调试注入。
pub(super) fn build(
    ui: &Rc<Ui>,
    cfg: &Config,
    index: usize,
    pin: &str,
    entry: AppEntry,
    slots: &Slots,
    editing: bool,
) -> (gtk::Button, Item, Option<gtk::Popover>) {
    let across = slots.across;
    let icon_name = if entry.icon.is_empty() {
        "application-x-executable".to_string()
    } else {
        entry.icon.clone()
    };
    let image = crate::icons::image_for(&icon_name, cfg.icon_size.clamp(8, 256));
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
    overlay.set_size_request(slots.w, slots.h);
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
            pin
        } else {
            entry.name.as_str()
        }));
    }
    button.add_css_class("dock-item");

    if editing {
        button.add_css_class("editing");
        super::edit::attach_drag(&button, index);
    }

    let motion = gtk::EventControllerMotion::new();
    {
        let u = ui.clone();
        let i = index;
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
        let i = index;
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
        let i = index;
        let multi_window = cfg.multi_window.clone();
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

            let has_window = !ids.is_empty();
            let running = !has_window && app_running(&e);
            match launch::action(&e.id, &multi_window, has_window, running) {
                Action::Launch => {
                    spawn_exec(&e.exec);
                    u.bounce(i);
                }
                Action::Nudge => u.bounce(i),
                Action::Focus => {
                    let next = ids
                        .iter()
                        .position(|(_, focused)| *focused)
                        .map(|p| ids[(p + 1) % ids.len()].0)
                        .unwrap_or(ids[0].0);
                    niri_focus(next);
                }
            }
        });
    }

    let item = Item {
        entry,
        button: button.clone(),
        image,
        dot,
        slot: overlay.clone(),
        level: std::cell::Cell::new(0),
    };

    if !cfg.context_menu || editing {
        return (button, item, None);
    }

    let pop = menu_for(ui, pin, &item, &button);
    (button, item, Some(pop))
}

/// 右键菜单：标题 + 新窗口 / 退出，指针离开 500ms 后自动收起
fn menu_for(ui: &Rc<Ui>, pin: &str, item: &Item, button: &gtk::Button) -> gtk::Popover {
    let entry = item.entry.clone();
    let pop = gtk::Popover::new();
    pop.add_css_class("dock-menu");
    // 自动关闭依赖 GTK 的键盘/指针 grab，而 layer surface 上 grab 不可靠，
    // 会出现「指针一进菜单就消失」，所以自己管生命周期
    pop.set_autohide(false);
    {
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let title = gtk::Label::new(Some(if entry.name.is_empty() {
            pin
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
        pop.set_parent(button);
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
    pop
}
