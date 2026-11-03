//! 托盘项（StatusNotifierItem）激活。
//!
//! 有些应用（QQ、Telegram）关掉窗口只是缩进托盘，进程和托盘项都还在。这时再启动
//! 一份会弹「重复登录」，所以点击应该让**已经存在的托盘项**把窗口叫回来——
//! 也就是替用户点一下托盘图标。托盘宿主（waybar 等）把注册过的项列在
//! `org.kde.StatusNotifierWatcher` 上，按 D-Bus 连接号问出 pid 就能对上应用。

use gtk4 as gtk;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

use crate::log_debug;

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";

/// 把某个进程的托盘项点一下；成功叫回窗口返回 `true`
pub(crate) fn activate(pid: i32) -> bool {
    let Some(conn) = session_bus() else {
        return false;
    };
    for (bus, path) in items(&conn) {
        let owner = owner_pid(&conn, &bus);
        log_debug!("tray", "托盘项 {bus}{path} 属于 pid {owner:?}");
        if owner != u32::try_from(pid).ok() {
            continue;
        }
        if call(
            &conn,
            &bus,
            &path,
            ITEM_IFACE,
            "Activate",
            Some(&(0i32, 0i32).to_variant()),
        ) {
            log_debug!("tray", "托盘激活成功 pid {pid}，窗口应该回来了");
            return true;
        }
    }
    log_debug!("tray", "pid {pid} 没有可用的托盘项");
    false
}

fn session_bus() -> Option<gio::DBusConnection> {
    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()
}

/// 托盘宿主里登记的项，形如 `:1.1088/StatusNotifierItem`
fn items(conn: &gio::DBusConnection) -> Vec<(String, String)> {
    let reply = conn
        .call_sync(
            Some(WATCHER),
            WATCHER_PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(
                &(
                    "org.kde.StatusNotifierWatcher",
                    "RegisteredStatusNotifierItems",
                )
                    .to_variant(),
            ),
            None,
            gio::DBusCallFlags::NONE,
            1_000,
            gio::Cancellable::NONE,
        )
        .ok();
    let Some(reply) = reply else {
        log_debug!("tray", "读取托盘列表失败");
        return Vec::new();
    };
    log_debug!("tray", "托盘列表回复类型 {}", reply.type_());
    let Some((value,)) = reply.get::<(glib::Variant,)>() else {
        log_debug!("tray", "回复里没有 variant");
        return Vec::new();
    };
    let Some(list) = value.get::<Vec<String>>() else {
        log_debug!("tray", "托盘列表类型 {:?} 解析失败", value.type_());
        return Vec::new();
    };
    list.iter().filter_map(|s| split_item(s)).collect()
}

/// `bus/path` 拆成两段；总线名里不会有 `/`，所以按第一个 `/` 切
fn split_item(item: &str) -> Option<(String, String)> {
    let idx = item.find('/')?;
    let (bus, path) = item.split_at(idx);
    if bus.is_empty() || path.len() < 2 {
        return None;
    }
    Some((bus.to_string(), path.to_string()))
}

/// 某个总线连接号属于哪个进程
fn owner_pid(conn: &gio::DBusConnection, bus: &str) -> Option<u32> {
    let reply = conn
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetConnectionUnixProcessID",
            Some(&(bus,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            1_000,
            gio::Cancellable::NONE,
        )
        .ok()?;
    reply.get::<(u32,)>().map(|(pid,)| pid)
}

fn call(
    conn: &gio::DBusConnection,
    bus: &str,
    path: &str,
    iface: &str,
    method: &str,
    params: Option<&glib::Variant>,
) -> bool {
    conn.call_sync(
        Some(bus),
        path,
        iface,
        method,
        params,
        None,
        gio::DBusCallFlags::NONE,
        1_000,
        gio::Cancellable::NONE,
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::split_item;

    #[test]
    fn splits_registered_items() {
        assert_eq!(
            split_item(":1.1088/StatusNotifierItem"),
            Some((":1.1088".into(), "/StatusNotifierItem".into()))
        );
        assert_eq!(
            split_item("org.freedesktop.StatusNotifierItem-508033-1/StatusNotifierItem"),
            Some((
                "org.freedesktop.StatusNotifierItem-508033-1".into(),
                "/StatusNotifierItem".into()
            ))
        );
        assert_eq!(split_item("/StatusNotifierItem"), None);
        assert_eq!(split_item(":1.1088"), None);
    }
}
