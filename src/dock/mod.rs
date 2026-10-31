//! dock 的窗口与交互。

mod build;
mod item;
mod ui;

pub(crate) use build::{build_ui, dump_matches};
pub(crate) use item::Item;
pub(crate) use ui::Ui;
