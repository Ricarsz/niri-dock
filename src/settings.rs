//! niri-dock 的设置窗口：所有改动直接写回 ~/.config/niri-dock/config.json，
//! dock 进程每 600ms 检查一次文件时间戳，改了会自动重建面板。

use crate::log_info;
use gtk4 as gtk;
use gtk::prelude::*;
use std::rc::Rc;

use crate::config::{load_config, save_config, save_pins, Config};

const CSS: &str = r#"
window { background-color: #1c1c20; }
.hero { font-size: 1.5em; font-weight: 700; }
.hero-sub { opacity: 0.55; font-size: 0.92em; }
.section-title {
    font-weight: 700;
    font-size: 0.82em;
    opacity: 0.55;
    margin: 2px 6px 6px 6px;
}
.row { padding: 12px 16px; }
.boxed-list {
    background-color: rgba(255, 255, 255, 0.045);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 14px;
}
.boxed-list > separator { opacity: 0.35; }
.row-title { font-weight: 600; }
.hint { font-size: 0.84em; opacity: 0.55; }
.spin { min-width: 96px; }
.dropdown { min-width: 132px; }
.footer-hint { opacity: 0.5; font-size: 0.85em; }
"#;

type Refresh = Rc<std::cell::RefCell<Option<Rc<dyn Fn()>>>>;

/// 「应用」分组：列出 dock 上的应用，可以移除，也可以从磁盘挑 .desktop 加进来
fn apps_section(window: &gtk::ApplicationWindow) -> gtk::Box {
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
                    open_add_dialog(&win, Rc::new(move || {
                        if let Some(cb) = f.borrow().as_ref() {
                            cb();
                        }
                    }));
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
fn open_add_dialog(parent: &gtk::ApplicationWindow, on_added: Rc<dyn Fn()>) {
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
        rows.borrow_mut().push((entry.id.clone(), entry.name.clone(), check));
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
            log_info!("settings", "添加 {} 个应用: {}", picked.len(), picked.join(", "));
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

/// 一行：左侧标题 + 说明，右侧控件
fn row(title: &str, hint: &str, suffix: &impl IsA<gtk::Widget>) -> gtk::Box {
    let name = gtk::Label::new(Some(title));
    name.set_xalign(0.0);
    name.add_css_class("row-title");

    let desc = gtk::Label::new(Some(hint));
    desc.set_xalign(0.0);
    desc.add_css_class("hint");
    desc.set_wrap(true);
    desc.set_max_width_chars(46);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&name);
    text.append(&desc);

    let line = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    line.add_css_class("row");
    line.append(&text);
    suffix.set_valign(gtk::Align::Center);
    line.append(suffix);
    line
}

fn section(title: &str, rows: &[gtk::Box]) -> gtk::Box {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.add_css_class("boxed-list");

    let head = gtk::Label::new(Some(title));
    head.set_xalign(0.0);
    head.add_css_class("section-title");

    for (i, r) in rows.iter().enumerate() {
        if i > 0 {
            r.set_has_tooltip(false);
            holder.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
        holder.append(r);
    }

    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
    wrap.append(&head);
    wrap.append(&holder);
    wrap
}

fn number(
    title: &str,
    hint: &str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    digits: u32,
    on_change: impl Fn(&mut Config, f64) + 'static,
) -> gtk::Box {
    let spin = gtk::SpinButton::with_range(min, max, step);
    spin.set_digits(digits);
    spin.set_value(value);
    spin.set_numeric(true);
    spin.add_css_class("spin");
    spin.set_halign(gtk::Align::End);

    let cb = Rc::new(on_change);
    spin.connect_value_changed(move |s| {
        let mut cfg = load_config();
        cb(&mut cfg, s.value());
        save_config(&cfg);
    });

    row(title, hint, &spin)
}

fn toggle(
    title: &str,
    hint: &str,
    value: bool,
    on_change: impl Fn(&mut Config, bool) + 'static,
) -> gtk::Box {
    let switch = gtk::Switch::new();
    switch.set_active(value);
    switch.set_halign(gtk::Align::End);

    let cb = Rc::new(on_change);
    switch.connect_active_notify(move |s| {
        let mut cfg = load_config();
        cb(&mut cfg, s.is_active());
        save_config(&cfg);
    });

    row(title, hint, &switch)
}

fn choice(
    title: &str,
    hint: &str,
    options: &[&str],
    selected: &str,
    on_change: impl Fn(&mut Config, String) + 'static,
) -> gtk::Box {
    let drop = gtk::DropDown::from_strings(options);
    let idx = options.iter().position(|o| *o == selected).unwrap_or(0) as u32;
    drop.set_selected(idx);
    drop.add_css_class("dropdown");
    drop.set_halign(gtk::Align::End);

    let cb = Rc::new(on_change);
    let opts: Vec<String> = options.iter().map(|s| s.to_string()).collect();
    drop.connect_selected_notify(move |d| {
        if let Some(value) = opts.get(d.selected() as usize) {
            let mut cfg = load_config();
            cb(&mut cfg, value.clone());
            save_config(&cfg);
        }
    });

    row(title, hint, &drop)
}

pub fn run() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("io.github.ricarsz.niridock.settings")
        .build();

    app.connect_activate(|app| {
        let cfg = load_config();

        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("niri-dock 设置")
            .default_width(640)
            .default_height(780)
            .build();

        let header = gtk::HeaderBar::new();
        let title = gtk::Label::new(Some("niri-dock"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));
        window.set_titlebar(Some(&header));

        let hero = gtk::Label::new(Some("dock 设置"));
        hero.set_xalign(0.0);
        hero.add_css_class("hero");

        let hero_sub = gtk::Label::new(Some(
            "每项改动即时写入 config.json，dock 会在 1 秒内重建面板。",
        ));
        hero_sub.set_xalign(0.0);
        hero_sub.set_wrap(true);
        hero_sub.add_css_class("hero-sub");

        let look = section(
            "外观",
            &[
                number(
                    "图标尺寸",
                    "图标本身的边长（像素）；只改图标，不改 dock 的长宽",
                    cfg.icon_size as f64,
                    16.0,
                    96.0,
                    1.0,
                    0,
                    |c, v| c.icon_size = v.round() as i32,
                ),
                number(
                    "图标槽位",
                    "每个图标占的格子边长（像素）；dock 的长宽由它决定",
                    cfg.slot as f64,
                    32.0,
                    112.0,
                    1.0,
                    0,
                    |c, v| c.slot = v.round() as i32,
                ),
                number(
                    "图标间距",
                    "相邻图标之间额外的空隙（像素）",
                    cfg.spacing as f64,
                    0.0,
                    40.0,
                    1.0,
                    0,
                    |c, v| c.spacing = v.round() as i32,
                ),
                number(
                    "面板内边距",
                    "面板边缘到图标之间的留白（像素）",
                    cfg.padding as f64,
                    0.0,
                    32.0,
                    1.0,
                    0,
                    |c, v| c.padding = v.round() as i32,
                ),
                number(
                    "面板圆角",
                    "面板的圆角半径（像素），22 接近 macOS",
                    cfg.panel_radius as f64,
                    0.0,
                    44.0,
                    1.0,
                    0,
                    |c, v| c.panel_radius = v.round() as i32,
                ),
                number(
                    "面板不透明度",
                    "1.00 完全不透明，0.00 全透明",
                    cfg.panel_opacity,
                    0.15,
                    1.0,
                    0.01,
                    2,
                    |c, v| c.panel_opacity = v,
                ),
                choice(
                    "主题",
                    "面板与指示点的配色",
                    &["dark", "light"],
                    &cfg.theme,
                    |c, v| c.theme = v,
                ),
            ],
        );

        let place = section(
            "位置",
            &[
                choice(
                    "停靠位置",
                    "dock 贴在哪条屏幕边上，图标会跟着换方向",
                    &["bottom", "top", "left", "right"],
                    &cfg.position,
                    |c, v| c.position = v,
                ),
                number(
                    "离屏幕边缘",
                    "面板和停靠边之间的距离（像素）",
                    cfg.gap as f64,
                    0.0,
                    64.0,
                    1.0,
                    0,
                    |c, v| c.gap = v.round() as i32,
                ),
            ],
        );

        let behavior = section(
            "行为",
            &[
                toggle(
                    "自动隐藏",
                    "鼠标离开后把面板滑到屏幕外，贴边滑回",
                    cfg.autohide,
                    |c, v| c.autohide = v,
                ),
                number(
                    "触发区高度",
                    "屏幕这条边上的触发带宽（像素），横贯整条边；鼠标碰到就弹出",
                    cfg.hot_zone as f64,
                    1.0,
                    32.0,
                    1.0,
                    0,
                    |c, v| c.hot_zone = v.round() as i32,
                ),
                number(
                    "动画时长",
                    "dock 滑出与收起用的时间（毫秒），大了更从容",
                    cfg.reveal_ms as f64,
                    60.0,
                    900.0,
                    20.0,
                    0,
                    |c, v| c.reveal_ms = v.round() as u64,
                ),
                number(
                    "隐藏延迟",
                    "鼠标离开后多久开始收起（毫秒）",
                    cfg.hide_delay_ms as f64,
                    0.0,
                    3000.0,
                    50.0,
                    0,
                    |c, v| c.hide_delay_ms = v.round() as u64,
                ),
                toggle(
                    "悬停放大",
                    "macOS 那种鼠标下方图标放大的效果",
                    cfg.magnify,
                    |c, v| c.magnify = v,
                ),
                toggle(
                    "悬停提示",
                    "鼠标停在图标上时显示应用名",
                    cfg.tooltip,
                    |c, v| c.tooltip = v,
                ),
                toggle(
                    "右键菜单",
                    "右键图标弹出「新窗口 / 退出」选项框",
                    cfg.context_menu,
                    |c, v| c.context_menu = v,
                ),
                number(
                    "放大峰值",
                    "悬停图标的放大倍数，1.30 比较克制",
                    cfg.magnify_peak,
                    1.0,
                    1.6,
                    0.01,
                    2,
                    |c, v| c.magnify_peak = v,
                ),
            ],
        );

        // 编辑模式入口：dock 进程负责真正进入编辑态
        let edit_btn = gtk::Button::with_label("编辑 dock 图标");
        edit_btn.set_tooltip_text(Some("进入后拖动图标就能调整顺序，点 ✓ 退出"));
        edit_btn.connect_clicked(|_| {
            let _ = crate::cli::activate_action("edit");
        });

        let reset = gtk::Button::with_label("恢复默认外观");
        reset.set_halign(gtk::Align::Start);
        {
            let window = window.clone();
            let app = app.clone();
            reset.connect_clicked(move |_| {
                let pins = load_config().pins;
                let mut def = Config::default();
                def.pins = pins; // 保留用户自己排的应用
                save_config(&def);
                window.close();
                app.activate();
            });
        }

        let footer = gtk::Label::new(Some(
            "应用列表在上方「应用」里增删，顺序在 dock 的编辑模式里拖动调整；恢复默认只重置外观与行为。",
        ));
        footer.set_xalign(0.0);
        footer.set_wrap(true);
        footer.add_css_class("footer-hint");

        let body = gtk::Box::new(gtk::Orientation::Vertical, 22);
        body.set_margin_top(20);
        body.set_margin_bottom(28);
        body.set_margin_start(20);
        body.set_margin_end(20);
        let apps = apps_section(&window);
        body.append(&hero);
        body.append(&hero_sub);
        body.append(&apps);
        body.append(&look);
        body.append(&place);
        body.append(&behavior);

        let footer_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        edit_btn.set_halign(gtk::Align::Start);
        footer_box.append(&edit_btn);
        footer_box.append(&reset);
        footer_box.append(&footer);
        body.append(&footer_box);

        let scroll = gtk::ScrolledWindow::builder()
            .child(&body)
            .vexpand(true)
            .build();

        window.set_child(Some(&scroll));
        window.present();

        // 调试用：直接打开「添加应用」多选框，方便截图
        if std::env::var("NIRI_DOCK_ADD_DIALOG").is_ok() {
            let noop: Rc<dyn Fn()> = Rc::new(|| {});
            open_add_dialog(&window, noop);
        }
    });

    // 忽略进程参数，否则 GTK 会把 --settings 当成未知选项
    app.run_with_args::<&str>(&[])
}
