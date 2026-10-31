//! 图标解析：先按名字走图标主题，失败再退回绝对路径 / 磁盘上按文件名找。
//!
//! 有些包的 `.desktop` 里 `Icon=` 写的是绝对路径（例如微信写的
//! `/usr/share/icons/hicolor/512x512/apps/wechat.png`），`from_icon_name`
//! 认不了这种写法，会退化成“图片缺失”的禁止符号。这里统一兜住。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gtk4 as gtk;

/// 按名字拿一个 GtkImage：主题 → 绝对路径 → 磁盘索引 → 主题兜底（缺失图标）
pub(crate) fn image_for(name: &str, size: i32) -> gtk::Image {
    let image = if let Some(path) = resolve(name) {
        gtk::Image::from_file(path)
    } else {
        gtk::Image::from_icon_name(name)
    };
    image.set_pixel_size(size);
    image
}

fn resolve(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    // 1. `Icon=` 直接写死的绝对路径
    if name.starts_with('/') {
        let path = PathBuf::from(name);
        return path.is_file().then_some(path);
    }
    // 2. 图标主题里有就用主题的（保留 symbolic 等语义）
    if let Some(display) = gtk::gdk::Display::default() {
        if gtk::IconTheme::for_display(&display).has_icon(name) {
            return None;
        }
    }
    // 3. 主题没有：磁盘上按文件名找一张
    index().get(name).cloned()
}

/// 首次使用时扫一遍常见图标目录，建立 文件名(无扩展名) → 路径 的索引。
fn index() -> &'static HashMap<String, PathBuf> {
    static INDEX: OnceLock<HashMap<String, PathBuf>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            roots.push(home.join(".local/share/icons"));
            roots.push(home.join(".icons"));
        }
        if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
            roots.push(PathBuf::from(data_home).join("icons"));
        }
        roots.push(PathBuf::from("/usr/local/share/icons"));
        roots.push(PathBuf::from("/usr/share/icons"));
        roots.push(PathBuf::from("/usr/local/share/pixmaps"));
        roots.push(PathBuf::from("/usr/share/pixmaps"));
        roots.push(PathBuf::from("/opt"));

        let mut map: HashMap<String, PathBuf> = HashMap::new();
        for root in roots {
            if root.is_dir() {
                walk(&root, 0, &mut map);
            }
        }
        map
    })
}

fn walk(dir: &Path, depth: usize, map: &mut HashMap<String, PathBuf>) {
    if depth > 5 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            walk(&path, depth + 1, map);
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "png" | "svg" | "xpm") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // 同一名字有多份时优先 png/svg、优先尺寸更大的目录名
        match map.get(stem) {
            Some(existing) => {
                if score(&path) > score(existing) {
                    map.insert(stem.to_string(), path);
                }
            }
            None => {
                map.insert(stem.to_string(), path);
            }
        }
    }
}

/// 粗略打分：png/svg 优于 xpm，路径里数字越大（尺寸越全）越优先
pub(crate) fn score(path: &Path) -> (u8, u32) {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let kind = match ext {
        "svg" => 3,
        "png" => 2,
        _ => 1,
    };
    let size = path
        .ancestors()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
        .find_map(|n| n.split('x').next().and_then(|s| s.parse::<u32>().ok()))
        .unwrap_or(0);
    (kind, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_prefers_svg_then_png_and_bigger_sizes() {
        assert!(score(Path::new("/icons/apps/foo.svg")) > score(Path::new("/icons/apps/foo.png")));
        assert!(score(Path::new("/icons/apps/foo.png")) > score(Path::new("/icons/apps/foo.xpm")));
        assert!(
            score(Path::new("/icons/512x512/apps/foo.png"))
                > score(Path::new("/icons/32x32/apps/foo.png"))
        );
    }
}
