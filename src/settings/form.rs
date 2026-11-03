//! 设置窗口的通用控件：一行、一组、数值 / 开关 / 下拉。

use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::config::{load_config, save_config, Config};

/// 一行：左侧标题 + 说明，右侧控件
pub(super) fn row(title: &str, hint: &str, suffix: &impl IsA<gtk::Widget>) -> gtk::Box {
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

pub(super) fn section(title: &str, rows: &[gtk::Box]) -> gtk::Box {
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

/// 一个数值设置项：范围 + 步长决定 SpinButton，小数位数由步长推出来
pub(super) fn number(
    title: &str,
    hint: &str,
    value: f64,
    range: std::ops::RangeInclusive<f64>,
    step: f64,
    on_change: impl Fn(&mut Config, f64) + 'static,
) -> gtk::Box {
    let spin = gtk::SpinButton::with_range(*range.start(), *range.end(), step);
    spin.set_digits(decimals(step));
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

/// 步长的小数位数：`1.0` → 0，`0.01` → 2
pub(super) fn decimals(step: f64) -> u32 {
    match format!("{step}").split_once('.') {
        Some((_, frac)) => frac.len() as u32,
        None => 0,
    }
}

pub(super) fn toggle(
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

pub(super) fn choice(
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

#[cfg(test)]
mod tests {
    use super::decimals;

    #[test]
    fn decimals_follow_the_step() {
        assert_eq!(decimals(1.0), 0);
        assert_eq!(decimals(20.0), 0);
        assert_eq!(decimals(0.5), 1);
        assert_eq!(decimals(0.01), 2);
    }
}
