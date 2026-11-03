//! 「应用」分组：列出 dock 上的应用、移除、多开开关，以及「添加应用」多选框。

use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::config::{load_config, save_multi_window, save_pins};
use crate::log_info;

type Refresh = Rc<std::cell::RefCell<Option<Rc<dyn Fn()>>>>;

/// 「应用」分组：列出 dock 上的应用，可以移除，也可以从磁盘挑 .desktop 加进来
pub(super) fn apps_section(window: &gtk::ApplicationWindow) -> gtk::Box {
    let cell: Refresh = Rc::new(std::cell::RefCell::new(None));
    let candidates: Rc<std::cell::RefCell<Vec<String>>> =
        Rc::new(std::cell::RefCell::new(Vec::new()));
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("boxed-list");

    let head = gtk::Label::new(Some("应用"));
    head.set_xalign(0.0);
    head.add_css_class("section-title");

    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
    wrap.append(&head);
    wrap.append(&list);

    {
        let cell2 = cell.clone();
        let list2 = list.clone();
        let candidates = candidates.clone();
        let window = window.clone();
        *cell.borrow_mut() = Some(Rc::new(move || {
            while let Some(child) = list2.first_child() {
                list2.remove(&child);
            }
            let cfg = load_config();
            let entries = crate::desktop::load_entries();
            for (i, pin) in cfg.pins.iter().enumerate() {
                if i > 0 {
                    list2.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                }
                let (name, icon) = entries
                    .get(pin)
                    .map(|e| (e.name.clone(), e.icon.clone()))
                    .unwrap_or_else(|| (pin.clone(), String::new()));
                let image = if icon.is_empty() {
                    gtk::Image::from_icon_name("application-x-executable")
                } else {
                    crate::icons::image_for(&icon, 24)
                };
                image.set_pixel_size(22);

                let label = gtk::Label::new(Some(&name));
                label.set_xalign(0.0);
                label.add_css_class("row-title");

                let file = gtk::Label::new(Some(&format!("{pin}.desktop")));
                file.set_xalign(0.0);
                file.add_css_class("hint");

                let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
                text.set_hexpand(true);
                text.append(&label);
                text.append(&file);

                let del = gtk::Button::with_label("移除");
                del.set_valign(gtk::Align::Center);

                // 多开：给终端这类「单实例、每次点开新窗口」的应用用
                let multi = gtk::Switch::new();
                multi.set_active(cfg.multi_window.iter().any(|m| m == pin));
                multi.set_valign(gtk::Align::Center);
                multi.set_tooltip_text(Some(
                    "已经启动时再点一次就开新窗口（Ghostty 这类终端用；QQ 这种别开）",
                ));
                {
                    let pin = pin.clone();
                    multi.connect_state_set(move |_, active| {
                        let cfg = load_config();
                        let mut ids = cfg.multi_window;
                        ids.retain(|m| m != &pin);
                        if active {
                            ids.push(pin.clone());
                        }
                        save_multi_window(&ids);
                        gtk::glib::Propagation::Proceed
                    });
                }
                let multi_label = gtk::Label::new(Some("多开"));
                multi_label.add_css_class("hint");
                let multi_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
                multi_box.set_valign(gtk::Align::Center);
                multi_box.append(&multi_label);
                multi_box.append(&multi);
                {
                    let pin = pin.clone();
                    let cell3 = cell2.clone();
                    del.connect_clicked(move |_| {
                        let cfg = load_config();
                        let pins: Vec<String> =
                            cfg.pins.into_iter().filter(|p| *p != pin).collect();
                        save_pins(&pins);
                        if let Some(f) = cell3.borrow().as_ref() {
                            f();
                        }
                    });
                }

                let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                line.add_css_class("row");
                line.append(&image);
                line.append(&text);
                line.append(&multi_box);
                line.append(&del);
                list2.append(&line);
            }

            if !cfg.pins.is_empty() {
                list2.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            }

            // 自动探测本机已安装的 .desktop：去重、隐藏的排除、按名字排序
            let available = crate::desktop::available_entries(&cfg.pins);
            *candidates.borrow_mut() = available.iter().map(|e| e.id.clone()).collect();

            let hint = if available.is_empty() {
                "没有可添加的应用（全部已添加，或被 NoDisplay / Hidden / TryExec 过滤）".to_string()
            } else {
                format!("本机已安装 {} 个可用应用，可以一次选多个", available.len())
            };
            let label = gtk::Label::new(Some(&hint));
            label.set_xalign(0.0);
            label.add_css_class("hint");

            let add = gtk::Button::with_label("添加应用…");
            add.set_halign(gtk::Align::Start);
            add.set_sensitive(!available.is_empty());
            {
                let cell3 = cell2.clone();
                let win = window.clone();
                add.connect_clicked(move |_| {
                    let f = cell3.clone();
                    open_add_dialog(
                        &win,
                        Rc::new(move || {
                            if let Some(cb) = f.borrow().as_ref() {
                                cb();
                            }
                        }),
                    );
                });
            }

            let add_row = gtk::Box::new(gtk::Orientation::Vertical, 8);
            add_row.add_css_class("row");
            add_row.append(&label);
            add_row.append(&add);
            list2.append(&add_row);
        }));
    }

    {
        let cell3 = cell.clone();
        gtk::glib::idle_add_local_once(move || {
            if let Some(f) = cell3.borrow().as_ref() {
                f();
            }
        });
    }

    wrap
}

/// 多选对话框：搜索 + 复选框列表，确认后一次性把选中的应用加进 pins
pub(super) fn open_add_dialog(parent: &gtk::ApplicationWindow, on_added: Rc<dyn Fn()>) {
    let cfg = load_config();
    let available = crate::desktop::available_entries(&cfg.pins);

    let dialog = gtk::Window::builder()
        .title("添加应用到 dock")
        .transient_for(parent)
        .modal(true)
        .default_width(560)
        .default_height(620)
        .build();

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("搜索应用名或 .desktop 文件名"));

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");

    let rows: Rc<std::cell::RefCell<Vec<(String, String, gtk::CheckButton)>>> =
        Rc::new(std::cell::RefCell::new(Vec::new()));

    for entry in &available {
        let check = gtk::CheckButton::new();
        let name = gtk::Label::new(Some(&entry.name));
        name.set_xalign(0.0);
        name.add_css_class("row-title");
        let file = gtk::Label::new(Some(&format!("{}.desktop", entry.id)));
        file.set_xalign(0.0);
        file.add_css_class("hint");
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.append(&name);
        text.append(&file);

        let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        line.add_css_class("row");
        line.append(&check);
        line.append(&text);
        list.append(&line);
        rows.borrow_mut()
            .push((entry.id.clone(), entry.name.clone(), check));
    }

    // 搜索只过滤，不重建列表
    {
        let rows = rows.clone();
        search.connect_search_changed(move |s| {
            let q = s.text().to_lowercase();
            for (id, name, check) in rows.borrow().iter() {
                let row = check.parent().and_then(|p| p.parent());
                if let Some(w) = row {
                    let hit = q.is_empty()
                        || name.to_lowercase().contains(&q)
                        || id.to_lowercase().contains(&q);
                    w.set_visible(hit);
                }
            }
        });
    }

    let cancel = gtk::Button::with_label("取消");
    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| d.close());
    }
    let confirm = gtk::Button::with_label("添加选中");
    confirm.add_css_class("suggested-action");
    {
        let d = dialog.clone();
        let rows = rows.clone();
        confirm.connect_clicked(move |_| {
            let picked: Vec<String> = rows
                .borrow()
                .iter()
                .filter(|(_, _, c)| c.is_active())
                .map(|(id, _, _)| id.clone())
                .collect();
            log_info!(
                "settings",
                "添加 {} 个应用: {}",
                picked.len(),
                picked.join(", ")
            );
            if !picked.is_empty() {
                let mut cfg = load_config();
                for id in picked {
                    if !cfg.pins.iter().any(|p| p == &id) {
                        cfg.pins.push(id);
                    }
                }
                save_pins(&cfg.pins);
                on_added();
            }
            d.close();
        });
    }

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    actions.append(&cancel);
    actions.append(&confirm);

    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .build();

    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_top(16);
    body.set_margin_bottom(16);
    body.set_margin_start(16);
    body.set_margin_end(16);
    body.append(&search);
    body.append(&scroll);
    body.append(&actions);
    dialog.set_child(Some(&body));
    dialog.present();
}
