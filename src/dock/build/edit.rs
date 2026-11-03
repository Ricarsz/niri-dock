//! 编辑模式：拖动排序、点 × 删除在设置里做、点 ✓ 退出。状态放在一个原子布尔里，
//! dock 重建后仍然生效（编辑过程中改配置会重建面板）。

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::cli::Rebuild;
use crate::config::{load_config, save_pins, Config};
use crate::log_debug;

use super::super::geometry::drop_index;

static EDITING: AtomicBool = AtomicBool::new(false);

pub(super) fn editing() -> bool {
    EDITING.load(Ordering::Relaxed)
}

pub(crate) fn toggle_editing() -> bool {
    let now = !editing();
    EDITING.store(now, Ordering::Relaxed);
    log_debug!("edit", "编辑模式 {}", if now { "开" } else { "关" });
    now
}

/// 拖动排序：拖自己，落到别的图标上就换位置
pub(super) fn attach_drag(button: &gtk::Button, index: usize) {
    let src = gtk::DragSource::new();
    src.set_actions(gtk::gdk::DragAction::MOVE);
    {
        let idx = index as u32;
        src.connect_prepare(move |_, _, _| {
            Some(gtk::gdk::ContentProvider::for_value(&idx.to_value()))
        });
    }
    button.add_controller(src);

    let dst = gtk::DropTarget::new(u32::static_type(), gtk::gdk::DragAction::MOVE);
    {
        let target = index;
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

fn reorder_pins(from: usize, to: usize) {
    let cfg = load_config();
    let mut pins = cfg.pins;
    if from >= pins.len() || to >= pins.len() || from == to {
        return;
    }
    let moved = pins.remove(from);
    log_debug!("edit", "移动 {moved}：{from} -> {to}");
    pins.insert(to, moved);
    save_pins(&pins);
}

/// 编辑模式下的「✓」按钮：退出编辑并让 dock 重建回正常面板
pub(super) fn done_button(cfg: &Config, slot_w: i32, slot_h: i32, rebuild: &Rebuild) -> gtk::Box {
    let tile = cfg.icon_size.clamp(8, 256) + 6;
    let done = gtk::Button::builder().has_frame(false).build();
    done.add_css_class("dock-item");
    done.add_css_class("dock-add");
    done.set_size_request(tile, tile);
    done.set_halign(gtk::Align::Center);
    done.set_valign(gtk::Align::Center);
    let label = gtk::Label::new(Some("✓"));
    label.set_markup(&format!(
        "<span font_size='{}pt'>✓</span>",
        cfg.icon_size.clamp(8, 256) / 2
    ));
    done.set_child(Some(&label));

    let overlay = gtk::Overlay::new();
    overlay.set_size_request(slot_w.max(24), slot_h.max(24));
    overlay.set_child(Some(&done));

    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.append(&overlay);

    {
        let r = rebuild.clone();
        done.connect_clicked(move |_| {
            crate::dock::toggle_editing();
            if let Some(f) = r.borrow().as_ref() {
                f();
            }
        });
    }
    holder
}
