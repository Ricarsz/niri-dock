//! niri-dock 的设置窗口：所有改动直接写回 ~/.config/niri-dock/config.json，
//! dock 进程用 inotify 盯着它，改了会自动重建面板。

mod form;
mod pins;

use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::config::{load_config, save_config, Config};

use form::{choice, number, section, toggle};

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
                    16.0..=96.0,
                    1.0,
                    |c, v| c.icon_size = v.round() as i32,
                ),
                number(
                    "图标槽位",
                    "每个图标占的格子边长（像素）；dock 的长宽由它决定",
                    cfg.slot as f64,
                    32.0..=112.0,
                    1.0,
                    |c, v| c.slot = v.round() as i32,
                ),
                number(
                    "图标间距",
                    "相邻图标之间额外的空隙（像素）",
                    cfg.spacing as f64,
                    0.0..=40.0,
                    1.0,
                    |c, v| c.spacing = v.round() as i32,
                ),
                number(
                    "面板内边距",
                    "面板边缘到图标之间的留白（像素）",
                    cfg.padding as f64,
                    0.0..=32.0,
                    1.0,
                    |c, v| c.padding = v.round() as i32,
                ),
                number(
                    "面板圆角",
                    "面板的圆角半径（像素），22 接近 macOS",
                    cfg.panel_radius as f64,
                    0.0..=44.0,
                    1.0,
                    |c, v| c.panel_radius = v.round() as i32,
                ),
                number(
                    "面板不透明度",
                    "1.00 完全不透明，0.00 全透明",
                    cfg.panel_opacity,
                    0.15..=1.0,
                    0.01,
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
                    0.0..=64.0,
                    1.0,
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
                    1.0..=32.0,
                    1.0,
                    |c, v| c.hot_zone = v.round() as i32,
                ),
                number(
                    "动画时长",
                    "dock 滑出与收起用的时间（毫秒），大了更从容",
                    cfg.reveal_ms as f64,
                    60.0..=900.0,
                    20.0,
                    |c, v| c.reveal_ms = v.round() as u64,
                ),
                number(
                    "隐藏延迟",
                    "鼠标离开后多久开始收起（毫秒）",
                    cfg.hide_delay_ms as f64,
                    0.0..=3000.0,
                    50.0,
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
                    1.0..=1.6,
                    0.01,
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
                // 保留用户自己排的应用，其余恢复默认
                let def = Config { pins, ..Default::default() };
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
        let apps = pins::apps_section(&window);
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
            pins::open_add_dialog(&window, noop);
        }
    });

    // 忽略进程参数，否则 GTK 会把 --settings 当成未知选项
    app.run_with_args::<&str>(&[])
}
