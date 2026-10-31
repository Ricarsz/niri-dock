//! niri-dock 的设置窗口：所有改动直接写回 ~/.config/niri-dock/config.json，
//! dock 进程每 600ms 检查一次文件时间戳，改了会自动重建面板。

use gtk4 as gtk;
use gtk::prelude::*;
use std::rc::Rc;

use crate::{load_config, save_config, Config};

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
        .application_id("com.ricarse.niridock.settings")
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
            "恢复默认只重置外观与行为，不动 dock 上的应用列表。",
        ));
        footer.set_xalign(0.0);
        footer.set_wrap(true);
        footer.add_css_class("footer-hint");

        let body = gtk::Box::new(gtk::Orientation::Vertical, 22);
        body.set_margin_top(20);
        body.set_margin_bottom(28);
        body.set_margin_start(20);
        body.set_margin_end(20);
        body.append(&hero);
        body.append(&hero_sub);
        body.append(&look);
        body.append(&place);
        body.append(&behavior);

        let footer_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        footer_box.append(&reset);
        footer_box.append(&footer);
        body.append(&footer_box);

        let scroll = gtk::ScrolledWindow::builder()
            .child(&body)
            .vexpand(true)
            .build();

        window.set_child(Some(&scroll));
        window.present();
    });

    // 忽略进程参数，否则 GTK 会把 --settings 当成未知选项
    app.run_with_args::<&str>(&[])
}
