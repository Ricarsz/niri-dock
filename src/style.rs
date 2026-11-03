//! 样式与放大曲线：把配置翻译成 GTK CSS，外加悬停放大的钟形衰减。

use gtk4 as gtk;

use crate::config::Config;
/// 放大档位：1.0 → 1.30，每档 0.02
pub(crate) const MAG_STEPS: usize = 15;
pub(crate) const MAG_STEP: f64 = 0.02;

/// 悬停放大：目标倍数按距离做钟形衰减
pub(crate) fn magnify_target(cfg: &Config, index: usize, hover: Option<usize>) -> f64 {
    if !cfg.magnify {
        return 1.0;
    }
    match hover {
        Some(h) => {
            let d = (index as f64 - h as f64).abs();
            let peak = cfg.magnify_peak.clamp(1.0, 1.30);
            1.0 + (peak - 1.0) * (-(d * d) / 0.9).exp()
        }
        None => 1.0,
    }
}

pub(crate) fn css_text(cfg: &Config) -> String {
    let light = cfg.theme.eq_ignore_ascii_case("light");
    let opacity = cfg.panel_opacity.clamp(0.0, 1.0);
    let (bg_r, bg_g, bg_b) = if light {
        (246.0, 246.0, 248.0)
    } else {
        (38.0, 38.0, 42.0)
    };
    // 圆角跟着图标尺寸走，小图标别留一个巨圆角
    let radius = cfg.panel_radius.max(0);
    let pad = cfg.padding.max(0);
    let (border, dot, _shadow, hover) = if light {
        (
            "rgba(0, 0, 0, 0.12)",
            "rgba(0, 0, 0, 0.55)",
            "rgba(0, 0, 0, 0.22)",
            "rgba(0, 0, 0, 0.08)",
        )
    } else {
        (
            "rgba(255, 255, 255, 0.13)",
            "rgba(255, 255, 255, 0.9)",
            "rgba(0, 0, 0, 0.45)",
            "rgba(255, 255, 255, 0.14)",
        )
    };

    let mut s = String::new();
    s.push_str(&format!(
        r#"
window, window.background {{
    background-color: transparent;
    background-image: none;
    box-shadow: none;
    border: none;
    outline: none;
}}

.dock-panel {{
    padding: {pad}px {pad}px;
    border-radius: {radius}px;
    border: 1px solid {border};
    background-color: rgba({bg_r}, {bg_g}, {bg_b}, {opacity});
    background-image: none;
}}

.dock-item {{
    padding: 0;
    margin: 0;
    border-radius: 14px;
    border: none;
    background-color: transparent;
    background-image: none;
    box-shadow: none;
    transition: transform 130ms cubic-bezier(0.25, 0.7, 0.3, 1);
    transform-origin: center bottom;
}}

.dock-item:active {{
    background-color: transparent;
}}

.dock-glow {{
    border-radius: 13px;
    background-color: transparent;
    transition: background-color 90ms ease;
}}

.dock-item:active .dock-glow {{
    background-color: {hover};
}}

/* 编辑模式：图标轻微晃动 + 删除角标 + 添加按钮 */
.dock-item.editing {{
    animation: dock-jiggle 320ms ease-in-out infinite;
}}
.dock-remove {{
    min-width: 16px;
    min-height: 16px;
    padding: 0;
    border-radius: 999px;
    background-color: rgba(0, 0, 0, 0.72);
    color: #ffffff;
    border: 1px solid rgba(255, 255, 255, 0.45);
    font-size: 11px;
    font-weight: 700;
}}
.dock-remove:hover {{
    background-color: #ff3b30;
    border-color: #ff3b30;
}}
.dock-add {{
    color: {dot};
    border-radius: 14px;
    background-color: rgba(127, 127, 127, 0.20);
}}
.dock-add:hover {{
    background-color: rgba(127, 127, 127, 0.34);
}}
@keyframes dock-jiggle {{
    0% {{ transform: rotate(-1.2deg); }}
    50% {{ transform: rotate(1.2deg); }}
    100% {{ transform: rotate(-1.2deg); }}
}}


.dot {{
    min-width: 4px;
    min-height: 4px;
    border-radius: 999px;
    background-color: {dot};
    transition: opacity 160ms ease;
}}

.dot.hidden {{
    opacity: 0;
}}

.dock-menu {{
    padding: 6px;
}}

.dock-menu .menu-title {{
    font-weight: 700;
    margin: 4px 10px 6px 10px;
}}

.dock-menu button {{
    padding: 8px 14px;
    border-radius: 10px;
    background-color: transparent;
    border: none;
    transition: background-color 90ms ease;
}}

.dock-menu button:hover {{
    background-color: rgba(255, 255, 255, 0.18);
}}

.dock-menu button:active {{
    background-color: rgba(255, 255, 255, 0.26);
}}
"#
    ));

    // 放大档位：只改 transform（不进布局），过渡由上面的 CSS transition 负责
    // 放大与弹跳都用 transform，同一个属性必须写进同一条规则里，
    // 所以放大档位 × 弹跳帧位生成组合类（GTK 的 transform 只认按钮这类节点的类）。
    let bounce = [(1usize, -14i32), (2, -2), (3, -8)];
    for i in 1..=MAG_STEPS {
        let scale = 1.0 + MAG_STEP * i as f64;
        s.push_str(&format!(
            ".dock-item.m{i} {{ transform: scale({scale:.3}); }}\n"
        ));
        for (n, y) in bounce {
            s.push_str(&format!(
                ".dock-item.m{i}.b{n} {{ transform: translate(0px, {y}px) scale({scale:.3}); }}\n"
            ));
        }
    }
    for (n, y) in bounce {
        s.push_str(&format!(
            ".dock-item.b{n} {{ transform: translate(0px, {y}px); }}\n"
        ));
    }
    s
}

thread_local! {
    static DOCK_CSS: gtk::CssProvider = gtk::CssProvider::new();
}

pub(crate) fn apply_css(cfg: &Config) {
    let css = css_text(cfg);
    DOCK_CSS.with(|provider| {
        provider.load_from_string(&css);
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                provider,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
        }
    });
}
