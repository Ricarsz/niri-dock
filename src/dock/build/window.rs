//! 图层窗口与面板外框：窗口属性、停靠边、Revealer、放大预留、折叠下限。

use gtk::prelude::*;
use gtk4 as gtk;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::config::Config;
use crate::dock::geometry::{room, slots, Slots};

/// 一个 dock 图层窗口的骨架：窗口本身、放图标的面板、滑动用的 Revealer
pub(super) struct Shell {
    pub(super) panel: gtk::Box,
    pub(super) revealer: gtk::Revealer,
    pub(super) slots: Slots,
}

/// 新建窗口，或复用配置重载前的那一个（重建 surface 会闪一下并多占显存）
pub(super) fn create(
    app: &gtk::Application,
    reuse: Option<gtk::ApplicationWindow>,
) -> gtk::ApplicationWindow {
    if let Some(window) = reuse {
        return window;
    }
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .decorated(false)
        .resizable(false)
        .build();
    window.init_layer_shell();
    window.set_namespace(Some("niri-dock"));
    window.set_layer(Layer::Top);
    window.set_margin(Edge::Bottom, 0);
    window.set_exclusive_zone(0);
    window.set_keyboard_mode(KeyboardMode::None);
    window
}

/// 按配置把面板、Revealer、透明底和窗口内容拼起来
pub(super) fn chrome(
    window: &gtk::ApplicationWindow,
    cfg: &Config,
    position: &str,
    horizontal: bool,
) -> Shell {
    let edge = match position {
        "top" => Edge::Top,
        "left" => Edge::Left,
        "right" => Edge::Right,
        _ => Edge::Bottom,
    };
    window.set_anchor(edge, true);
    window.set_margin(edge, 0);

    let panel = gtk::Box::new(
        if horizontal {
            gtk::Orientation::Horizontal
        } else {
            gtk::Orientation::Vertical
        },
        0,
    );
    panel.add_css_class("dock-panel");
    match edge {
        Edge::Top => panel.set_margin_top(cfg.gap.max(0)),
        Edge::Left => panel.set_margin_start(cfg.gap.max(0)),
        Edge::Right => panel.set_margin_end(cfg.gap.max(0)),
        _ => panel.set_margin_bottom(cfg.gap.max(0)),
    }

    let transition = match position {
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

    let slots = slots(horizontal, cfg.slot, cfg.spacing);
    let room = room(
        slots.across,
        cfg.icon_size,
        cfg.padding,
        cfg.magnify,
        cfg.magnify_peak,
    );

    let pad_cross = gtk::Box::new(
        if horizontal {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        },
        0,
    );
    if horizontal {
        pad_cross.set_size_request(-1, room);
    } else {
        pad_cross.set_size_request(room, -1);
    }

    let line = gtk::Box::new(
        if horizontal {
            gtk::Orientation::Horizontal
        } else {
            gtk::Orientation::Vertical
        },
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
        if horizontal {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        },
        0,
    );
    // 让出面板朝屏幕内侧的生长方向
    if matches!(position, "top" | "left") {
        wrap.append(&line);
        wrap.append(&pad_cross);
    } else {
        wrap.append(&pad_cross);
        wrap.append(&line);
    }

    revealer.set_child(Some(&wrap));

    // 热区就是这个窗口自己：折叠时窗口被钳到 hot_zone 大小（见 Ui::hide 的尺寸请求），
    // 内容全透明、什么都不画，指针碰到这条带就展开。旧版另开一条横贯屏幕的热区窗口，
    // 两个 surface 在屏幕边重叠、互相抢指针的 enter/leave，会自激抖动，所以删掉。
    let outer = gtk::Box::new(
        if horizontal {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        },
        0,
    );
    outer.append(&revealer);

    // 一层始终存在、alpha=0 的底色。GTK 只有在真的产生绘制命令时才会给窗口
    // 提交新 buffer；折叠时整棵树都隐藏（没有像素可画），它就不提交，于是
    // gtk4-layer-shell 不会把 layer surface 缩到 hot_zone，合成器继续显示
    // 收起动画最后一帧（30px 左右），屏幕底部就残留一条面板边缘的白线。
    // 这层透明底让窗口总有内容可画，折叠后 surface 能正常同步到 hot_zone。
    let backdrop = gtk::DrawingArea::new();
    backdrop.set_hexpand(true);
    backdrop.set_vexpand(true);
    backdrop.set_draw_func(|_, cr, _w, _h| {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        let _ = cr.paint();
    });

    outer.prepend(&backdrop);
    // 给窗口内容一条下限：收起动画会把 revealer 的 natural 缩到 0，窗口跟着掉到
    // 0 尺寸时 gtk4-layer-shell 会退回它 200px 的默认尺寸，热区瞬间变成屏幕底部
    // 200px 高的一条带——指针停在屏幕底往上一点的地方就会 enter→show→hide 循环。
    let hot = cfg.hot_zone.clamp(1, 32);
    if horizontal {
        outer.set_size_request(-1, hot);
    } else {
        outer.set_size_request(hot, -1);
    }
    window.set_child(Some(&outer));

    Shell {
        panel,
        revealer,
        slots,
    }
}
