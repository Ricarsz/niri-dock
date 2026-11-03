//! 点 dock 图标时该做什么。
//!
//! 只有判断，没有副作用：真正的启动、聚焦、回弹分别在 `build.rs` 里做，
//! 所以这里的规则可以直接单测，不用搭 GTK。

/// 点击一个图标的结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// 启动一个新实例
    Launch,
    /// 切到已经打开的窗口（有多个时在它们之间轮换）
    Focus,
    /// 应用在后台跑着但没有窗口（比如 QQ、Telegram 缩到托盘）：让托盘项把窗口叫回来
    Nudge,
}

/// 点图标时做什么：
///
/// - 勾了「多开」的应用：每点一次都开新实例——终端这类应用用单实例模式
///   就能在已有进程里开新窗口
/// - 没勾：有窗口就聚焦；没窗口但进程在跑就只回弹（避免 QQ 那种「重复登录」）；
///   都没命中才启动
pub(crate) fn action(
    pin: &str,
    multi_window: &[String],
    has_window: bool,
    running: bool,
) -> Action {
    if is_multi_window(pin, multi_window) {
        return Action::Launch;
    }
    match (has_window, running) {
        (true, _) => Action::Focus,
        (false, true) => Action::Nudge,
        (false, false) => Action::Launch,
    }
}

pub(crate) fn is_multi_window(pin: &str, multi_window: &[String]) -> bool {
    multi_window.iter().any(|m| m == pin)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn multi(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn multi_window_apps_always_launch() {
        let cfg = multi(&["com.mitchellh.ghostty"]);
        assert_eq!(
            action("com.mitchellh.ghostty", &cfg, true, true),
            Action::Launch
        );
        assert_eq!(
            action("com.mitchellh.ghostty", &cfg, false, false),
            Action::Launch
        );
    }

    #[test]
    fn single_window_apps_focus_when_a_window_exists() {
        assert_eq!(
            action("linuxqq-wayland-fix", &[], true, true),
            Action::Focus
        );
        // 有窗口时进程状态无关紧要
        assert_eq!(
            action("linuxqq-wayland-fix", &[], true, false),
            Action::Focus
        );
    }

    #[test]
    fn running_without_window_only_nudges() {
        // QQ 退到托盘：进程在、窗口没了，这时不能再启动一份
        assert_eq!(
            action("linuxqq-wayland-fix", &[], false, true),
            Action::Nudge
        );
    }

    #[test]
    fn not_running_launches() {
        assert_eq!(action("firefox", &[], false, false), Action::Launch);
    }

    #[test]
    fn multi_window_matches_exact_ids_only() {
        let cfg = multi(&["ghostty"]);
        assert!(is_multi_window("ghostty", &cfg));
        assert!(!is_multi_window("ghostty-nightly", &cfg));
        assert!(!is_multi_window("com.mitchellh.ghostty", &cfg));
    }
}
