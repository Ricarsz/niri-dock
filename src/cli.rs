//! 命令行子命令：启动/停止/重启/状态/显示隐藏/重载/设置/调试。
//!
//! 设计：dock 本体仍然是一个 GApplication 单实例；这里的子命令要么管理进程
//! （start/stop/restart/status），要么通过 D-Bus 调用 dock 注册的 GAction
//! （toggle/show/hide/reload/quit），不引入额外的守护进程或 socket 协议。

use std::io::Write;
use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use gtk4 as gtk;
use gtk::gio;
use gtk::prelude::*;

pub(crate) const APP_ID: &str = "io.github.ricarsz.niridock";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cmd {
    Run,
    Settings,
    Dump,
    Start,
    Stop,
    Restart,
    Status,
    Toggle,
    Show,
    Hide,
    Reload,
    Edit,
    Logs,
    Quit,
    Version,
    Help,
}

/// 把输出写到 stdout，管道被提前关闭时安静退出（避免 panic on broken pipe）
macro_rules! say {
    ($($arg:tt)*) => {{
        let mut out = std::io::stdout();
        let _ = writeln!(out, $($arg)*);
        let _ = out.flush();
    }};
}

/// 让 `niri-dock status | head` 这类管道正常退出，而不是 panic on broken pipe
pub(crate) fn restore_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

pub(crate) fn parse(args: &[String]) -> Cmd {
    match args.first().map(|s| s.as_str()) {
        None => Cmd::Run,
        Some("start") => Cmd::Start,
        Some("stop") => Cmd::Stop,
        Some("restart") => Cmd::Restart,
        Some("status") => Cmd::Status,
        Some("toggle") => Cmd::Toggle,
        Some("show") => Cmd::Show,
        Some("hide") => Cmd::Hide,
        Some("reload") => Cmd::Reload,
        Some("edit") => Cmd::Edit,
        Some("logs") | Some("log") => Cmd::Logs,
        Some("quit") => Cmd::Quit,
        Some("settings") | Some("--settings") => Cmd::Settings,
        Some("dump") | Some("--dump") => Cmd::Dump,
        Some("--version") | Some("-V") => Cmd::Version,
        Some("--help") | Some("-h") | Some("help") => Cmd::Help,
        Some(other) => {
            eprintln!("niri-dock: 未知命令 {other}\n");
            Cmd::Help
        }
    }
}

pub(crate) fn pid_path() -> PathBuf {
    let dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    dir.join("niri-dock.pid")
}

/// dock 启动时写 pid 文件，退出时清掉
pub(crate) fn write_pid() {
    let _ = std::fs::write(pid_path(), std::process::id().to_string());
}

pub(crate) fn clear_pid() {
    let _ = std::fs::remove_file(pid_path());
}

fn read_pid() -> Option<i32> {
    std::fs::read_to_string(pid_path())
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
}

fn alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // /proc 存在且在跑，且进程名匹配
    let path = format!("/proc/{pid}/comm");
    match std::fs::read_to_string(path) {
        Ok(name) => name.trim() == "niri-dock",
        Err(_) => false,
    }
}

pub(crate) fn running_pid() -> Option<i32> {
    read_pid().filter(|pid| alive(*pid))
}

fn app_path() -> String {
    format!("/{}", APP_ID.replace('.', "/"))
}

/// 通过 D-Bus 激活 dock 注册的 GAction；返回是否成功
pub(crate) fn activate_action(name: &str) -> bool {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return false;
    };
    let Ok(params) = glib::Variant::parse(
        None,
        &format!("('{name}', @av [], @a{{sv}} {{}})"),
    ) else {
        return false;
    };
    conn.call_sync(
        Some(APP_ID),
        &app_path(),
        "org.gtk.Actions",
        "Activate",
        Some(&params),
        None,
        gio::DBusCallFlags::NONE,
        1000,
        gio::Cancellable::NONE,
    )
    .is_ok()
}

/// 后台拉起 dock（已在运行则激活现有实例）
fn spawn_detached() -> bool {
    let Ok(exe) = env::current_exe() else {
        return false;
    };
    let home = env::var_os("HOME").map(PathBuf::from);
    // dock 自己会把日志写进 `~/.local/state/niri-dock/dock.log`，这里不用再重定向
    let mut cmd = Command::new("setsid");
    cmd.arg(exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(home) = home {
        cmd.current_dir(home);
    }
    // 别把 agent shell 的脏环境带进去
    for var in ["NO_COLOR", "TERM", "COLORTERM", "CLICOLOR", "CLICOLOR_FORCE"] {
        cmd.env_remove(var);
    }
    cmd.spawn().is_ok()
}

fn stop_running() -> bool {
    let Some(pid) = running_pid() else {
        clear_pid();
        return false;
    };
    let _ = Command::new("kill").arg("-TERM").arg(pid.to_string()).status();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !alive(pid) {
            clear_pid();
            return true;
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    let _ = Command::new("kill").arg("-KILL").arg(pid.to_string()).status();
    clear_pid();
    true
}

/// 往日志文件追加一行（CLI 是短命进程，stderr 不会留在文件里）
fn note_to_log(msg: &str) {
    let path = crate::log::log_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}  {:5} {:<8} {msg}", crate::log::stamp(), "info", "cli");
    }
}

/// `niri-dock logs`：打印日志文件路径和最后 120 行
fn print_logs() {
    let path = crate::log::log_file();
    say!("日志 {}", path.display());
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let lines: Vec<&str> = text.lines().collect();
            let start = lines.len().saturating_sub(120);
            for line in &lines[start..] {
                say!("{line}");
            }
        }
        Err(err) => say!("读不到日志: {err}"),
    }
}

fn print_status() {
    let cfg_path = crate::config::config_path();
    match running_pid() {
        Some(pid) => {
            say!("运行中 (pid {pid})");
        }
        None => say!("未运行"),
    }
    say!("版本    {}", env!("CARGO_PKG_VERSION"));
    say!("配置    {}", cfg_path.display());
    say!("日志    {}（niri-dock logs）", crate::log::log_file().display());
    let cfg = crate::config::load_config();
    say!(
        "外观    icon {} / slot {} / spacing {} / {}, {}",
        cfg.icon_size, cfg.slot, cfg.spacing, cfg.position, cfg.theme
    );
    say!("图标    {} 个", cfg.pins.len());
}

pub(crate) fn print_help() {
    say!(
        "niri-dock {} —— niri 的 macOS 风 dock\n\
         \n\
         用法: niri-dock [命令]\n\
         \n\
         不带命令      前台运行 dock（已在运行则激活）\n\
         start         后台启动 dock\n\
         stop          停止运行中的 dock\n\
         restart       重启 dock\n\
         status        查看是否在跑、版本、配置路径\n\
         toggle        显示 / 隐藏 dock\n\
         show          显示 dock\n\
         hide          隐藏 dock\n\
         reload        重新读取配置并重建面板
         edit          进入 / 退出图标编辑模式（拖动排序、加删图标）\n\
         quit          退出 dock\n\
         settings      打开设置窗口\n\
         dump          打印每个图标匹配到的窗口\n\
         help          显示本帮助\n\
         \n\
         环境变量   NIRI_DOCK_HOVER / NIRI_DOCK_BOUNCE / NIRI_DOCK_MENU（截图调试用）",
        env!("CARGO_PKG_VERSION")
    );
}

/// 执行非阻塞类子命令；返回 None 表示需要进入 dock 主循环
pub(crate) fn dispatch(cmd: Cmd) -> Option<glib::ExitCode> {
    match cmd {
        Cmd::Run => None,
        Cmd::Settings => Some(crate::settings::run()),
        Cmd::Dump => {
            crate::dock::dump_matches();
            Some(glib::ExitCode::SUCCESS)
        }
        Cmd::Version => {
            say!("niri-dock {}", env!("CARGO_PKG_VERSION"));
            Some(glib::ExitCode::SUCCESS)
        }
        Cmd::Help => {
            print_help();
            Some(glib::ExitCode::SUCCESS)
        }
        Cmd::Status => {
            print_status();
            Some(glib::ExitCode::SUCCESS)
        }
        Cmd::Logs => {
            print_logs();
            Some(glib::ExitCode::SUCCESS)
        }
        Cmd::Start => {
            note_to_log("start");
            if running_pid().is_some() {
                let _ = activate_action("show");
                say!("已在运行");
            } else if spawn_detached() {
                say!("已启动");
            } else {
                eprintln!("启动失败");
                return Some(glib::ExitCode::FAILURE);
            }
            Some(glib::ExitCode::SUCCESS)
        }
        Cmd::Stop | Cmd::Quit => {
            note_to_log("stop");
            if stop_running() {
                say!("已停止");
            } else {
                say!("未在运行");
            }
            Some(glib::ExitCode::SUCCESS)
        }
        Cmd::Restart => {
            stop_running();
            if spawn_detached() {
                say!("已重启");
                Some(glib::ExitCode::SUCCESS)
            } else {
                eprintln!("启动失败");
                Some(glib::ExitCode::FAILURE)
            }
        }
        Cmd::Toggle | Cmd::Show | Cmd::Hide | Cmd::Reload | Cmd::Edit => {
            let action = match cmd {
                Cmd::Toggle => "toggle",
                Cmd::Show => "show",
                Cmd::Hide => "hide",
                Cmd::Edit => "edit",
                _ => "reload",
            };
            note_to_log(action);
            if running_pid().is_none() {
                eprintln!("dock 没在运行（先 niri-dock start）");
                return Some(glib::ExitCode::FAILURE);
            }
            if activate_action(action) {
                Some(glib::ExitCode::SUCCESS)
            } else {
                eprintln!("调用 dock 失败：{action}");
                Some(glib::ExitCode::FAILURE)
            }
        }
    }
}

/// 在 dock 进程里注册可被 CLI 触发的 GAction
pub(crate) type Rebuild = std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<dyn Fn()>>>>;

pub(crate) fn register_actions(
    app: &gtk::Application,
    ui: &std::rc::Rc<crate::dock::Ui>,
    rebuild: &Rebuild,
) {
    let add = |name: &str, f: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| f());
        app.add_action(&action);
    };

    let app_quit = app.clone();
    add("quit", Box::new(move || app_quit.quit()));

    let u = ui.clone();
    add("show", Box::new(move || u.show()));

    let u = ui.clone();
    add("hide", Box::new(move || u.hide()));

    let u = ui.clone();
    add(
        "toggle",
        Box::new(move || {
            if u.shown.get() {
                u.hide();
            } else {
                u.show();
            }
        }),
    );

    let u = ui.clone();
    {
        let r = rebuild.clone();
        add(
            "reload",
            Box::new(move || {
                u.refresh_dots();
                if let Some(f) = r.borrow().as_ref() {
                    f();
                }
            }),
        );
    }

    // 图标编辑模式：开关后让 dock 重建面板，编辑控件在 build_ui 里按状态生成
    {
        let u = ui.clone();
        let r = rebuild.clone();
        add(
            "edit",
            Box::new(move || {
                let now = crate::dock::toggle_editing();
                // 直接重建面板：编辑控件由 build_ui 按 EDITING 生成
                if let Some(f) = r.borrow().as_ref() {
                    f();
                }
                // 编辑模式下别自动隐藏，否则一松手就滑走了
                if now {
                    u.show();
                }
            }),
        );
    }
}

