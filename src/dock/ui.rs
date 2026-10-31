//! dock 状态机：显示/隐藏、运行圆点、放大类、启动弹跳。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gtk4 as gtk;
use gtk::prelude::*;

use crate::config::Config;
use crate::desktop::*;
use crate::niri::*;
use crate::style::*;

use super::Item;

#[allow(dead_code)]
pub(crate) struct Ui {
    pub(crate) cfg: Config,
    pub(crate) entries: Rc<HashMap<String, AppEntry>>,
    pub(crate) windows: Arc<Mutex<HashMap<i64, Win>>>,
    pub(crate) epoch: Arc<AtomicU64>,
    pub(crate) items: RefCell<Vec<Item>>,
    pub(crate) hover: std::cell::Cell<Option<usize>>,
    pub(crate) revealer: gtk::Revealer,
    pub(crate) panel: gtk::Box,
    pub(crate) window: gtk::ApplicationWindow,
    pub(crate) shown: Rc<std::cell::Cell<bool>>,
    pub(crate) hide_at: RefCell<Option<Instant>>,
    /// 右键菜单展开的数量：菜单开着时不能隐藏 dock，否则菜单跟着一起消失
    pub(crate) menu_open: std::cell::Cell<usize>,
    pub(crate) popovers: RefCell<Vec<gtk::Popover>>,
    pub(crate) menu_close_at: RefCell<Option<Instant>>,
}

impl Ui {
    pub(crate) fn window_for(&self, entry: &AppEntry) -> Option<i64> {
        let keys = match_keys(entry);
        let map = self.windows.lock().unwrap();
        let mut best: Option<&Win> = None;
        for w in map.values() {
            if !app_matches(&keys, &w.app_id) {
                continue;
            }
            best = match best {
                None => Some(w),
                Some(b) => {
                    if w.is_focused && !b.is_focused {
                        Some(w)
                    } else {
                        Some(b)
                    }
                }
            };
        }
        best.map(|w| w.id)
    }

    pub(crate) fn refresh_dots(&self) {
        for item in self.items.borrow().iter() {
            let running = self.window_for(&item.entry).is_some();
            if running {
                item.dot.remove_css_class("hidden");
            } else {
                item.dot.add_css_class("hidden");
            }
        }
    }

    /// 关闭所有右键菜单（切换/隐藏 dock 时用）
    pub(crate) fn close_menus(&self) {
        for p in self.popovers.borrow().iter() {
            if p.is_visible() {
                p.popdown();
            }
        }
    }

    pub(crate) fn show(&self) {
        if self.shown.get() {
            return;
        }
        self.revealer.set_visible(true);
        self.revealer.set_reveal_child(true);
        self.shown.set(true);
        *self.hide_at.borrow_mut() = None;
    }

    pub(crate) fn hide(&self) {
        if !self.shown.get() || self.menu_open.get() > 0 {
            return;
        }
        self.close_menus();
        self.revealer.set_reveal_child(false);
        self.shown.set(false);
        // 收完之后把 revealer 整个藏掉：它自己会留 1px，那正是屏幕底那条细线
        let revealer = self.revealer.clone();
        let window = self.window.clone();
        let shown = self.shown.clone();
        glib::timeout_add_local_once(Duration::from_millis(260), move || {
            if !shown.get() {
                revealer.set_visible(false);
                // 窗口缩小后 GTK 只会重画脏区，最上面那 1px 会留着上一帧的面板边框
                window.queue_draw();
            }
        });
    }

    pub(crate) fn schedule_hide(&self) {
        if !self.cfg.autohide {
            return;
        }
        *self.hide_at.borrow_mut() =
            Some(Instant::now() + Duration::from_millis(self.cfg.hide_delay_ms));
    }

    /// 悬停变化时只切 CSS 类：真正插值由 GTK 的 CSS transition 在合成层上跑，
    /// 不需要每帧回调，也不改布局（槽位尺寸固定）。
    pub(crate) fn apply_magnification(&self) {
        let hover = self.hover.get();
        for (i, item) in self.items.borrow().iter().enumerate() {
            let scale = magnify_target(&self.cfg, i, hover);
            let level = (((scale - 1.0) / MAG_STEP).round() as usize).min(MAG_STEPS);
            if item.level.get() != level {
                if item.level.get() > 0 {
                    item.button.remove_css_class(&format!("m{}", item.level.get()));
                }
                if level > 0 {
                    item.button.add_css_class(&format!("m{level}"));
                }
                item.level.set(level);
            }
        }
    }

    /// 启动弹跳：只切 CSS 类（translateY），过渡交给合成层。
    /// 不能用改 margin / pixel_size 的方式，那会触发布局与重绘，弹跳会一顿一顿。
    pub(crate) fn bounce(&self, idx: usize) {
        let button = match self.items.borrow().get(idx) {
            Some(item) => item.button.clone(),
            None => return,
        };
        for (delay, class) in [(0u64, "b1"), (130, "b2"), (250, "b3"), (380, "")] {
            let btn = button.clone();
            glib::timeout_add_local_once(Duration::from_millis(delay), move || {
                for c in ["b1", "b2", "b3"] {
                    btn.remove_css_class(c);
                }
                if !class.is_empty() {
                    btn.add_css_class(class);
                }
            });
        }
    }
}

