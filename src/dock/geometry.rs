//! 面板的纯几何：格子大小、放大预留、拖放落点、折叠尺寸。
//!
//! 这些函数不碰 GTK，改起来可以先用测试锁住行为，再动 dock 的构建代码。

/// 图标在槽位里偏下放置时上下留白的分配
pub(crate) fn item_bottom_margin(slot: i32, icon: i32) -> i32 {
    ((slot - icon) / 2).clamp(0, 24) + 2
}

/// 一个图标占的槽位：沿停靠边的方向是 `slot + spacing`，横向就是 `slot`
pub(crate) struct Slots {
    /// 槽位的横向边长，放大预留按它算
    pub(crate) across: i32,
    /// 分给 Overlay 的宽度
    pub(crate) w: i32,
    /// 分给 Overlay 的高度
    pub(crate) h: i32,
}

pub(crate) fn slots(horizontal: bool, slot: i32, spacing: i32) -> Slots {
    let across = slot.clamp(16, 160);
    let along = across + spacing.max(0);
    let (w, h) = if horizontal {
        (along, across)
    } else {
        (across, along)
    };
    Slots { across, w, h }
}

/// 面板两侧要留出多少空间：放大后的图标、以及比格子还大的图标都会冒出面板
pub(crate) fn room(across: i32, icon_size: i32, padding: i32, magnify: bool, peak: f64) -> i32 {
    // GTK 的 transform-origin 不生效，缩放以按钮中心为原点，所以最多向上长
    // 格子 × (峰值 - 1)
    let growth = if magnify {
        (across as f64 * (peak - 1.0)).ceil() as i32 + 8
    } else {
        0
    };
    let tall_icon =
        (icon_size + item_bottom_margin(across, icon_size) - (across + padding.max(0))).max(0);
    growth.max(tall_icon)
}

/// 折叠态的窗口尺寸请求：沿停靠边的方向只留 hot_zone，其余交给内容
pub(crate) fn collapsed_request(position: &str, hot_zone: i32) -> (i32, i32) {
    let hot = hot_zone.clamp(1, 32);
    if matches!(position.to_ascii_lowercase().as_str(), "left" | "right") {
        (hot, -1)
    } else {
        (-1, hot)
    }
}

/// 拖放落点：落在目标图标右半边就插到它后面（from 先被移除，索引要相应回退一格）
pub(crate) fn drop_index(from: usize, target: usize, right_half: bool) -> usize {
    let mut to = if right_half { target + 1 } else { target };
    if from < to {
        to -= 1;
    }
    to
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margin_keeps_the_icon_low_but_bounded() {
        assert_eq!(item_bottom_margin(65, 60), 4);
        assert_eq!(item_bottom_margin(40, 60), 2);
        assert_eq!(item_bottom_margin(120, 20), 26);
    }

    #[test]
    fn slots_follow_the_dock_edge() {
        let h = slots(true, 60, 4);
        assert_eq!((h.across, h.w, h.h), (60, 64, 60));
        let v = slots(false, 60, 4);
        assert_eq!((v.across, v.w, v.h), (60, 60, 64));
        // 超出范围的槽位被钳住
        assert_eq!(slots(true, 8, 0).across, 16);
        assert_eq!(slots(true, 400, 0).across, 160);
        // 间距为负按 0 算
        assert_eq!(slots(true, 60, -10).w, 60);
    }

    #[test]
    fn room_only_grows_when_something_overflows() {
        // 图标比格子小：不需要预留
        assert_eq!(room(65, 60, 8, false, 1.3), 0);
        // 放大开来要留出向上的余量：ceil(65 * 0.3) + 8
        assert_eq!(room(65, 60, 8, true, 1.3), 28);
        // 图标比格子大：超出部分要留出来
        assert_eq!(room(40, 60, 0, false, 1.3), 22);
    }

    #[test]
    fn collapsed_request_keeps_the_hot_zone() {
        assert_eq!(collapsed_request("bottom", 4), (-1, 4));
        assert_eq!(collapsed_request("top", 40), (-1, 32));
        assert_eq!(collapsed_request("left", 4), (4, -1));
        assert_eq!(collapsed_request("RIGHT", 1), (1, -1));
    }

    #[test]
    fn drop_index_moves_forward() {
        // 0 -> 2 的右半边：插到 2 之后，移除 0 后是 2
        assert_eq!(drop_index(0, 2, true), 2);
        // 0 -> 2 的左半边：插到 2 之前，移除 0 后是 1
        assert_eq!(drop_index(0, 2, false), 1);
    }

    #[test]
    fn drop_index_moves_backward() {
        assert_eq!(drop_index(3, 1, false), 1);
        assert_eq!(drop_index(3, 1, true), 2);
    }

    #[test]
    fn drop_index_stays_put() {
        assert_eq!(drop_index(1, 1, false), 1);
    }
}
