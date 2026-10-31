//! 单个 dock 图标的数据：按钮、图标、槽位、运行圆点、右键菜单。

use gtk4 as gtk;

use crate::desktop::AppEntry;

#[allow(dead_code)]
pub(crate) struct Item {
    pub(crate) entry: AppEntry,
    pub(crate) button: gtk::Button,
    pub(crate) image: gtk::Image,
    pub(crate) dot: gtk::Box,
    pub(crate) slot: gtk::Overlay,
    pub(crate) level: std::cell::Cell<usize>,
}

