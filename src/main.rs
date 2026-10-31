//! niri-dock —— 给 niri 用的 macOS 风 dock（GTK4 + gtk4-layer-shell + niri IPC）

mod config;
mod desktop;
mod dock;
mod niri;
mod settings;
mod style;

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtk4 as gtk;
use gtk::prelude::*;

use config::{config_mtime, load_config};
use desktop::load_entries;
use dock::{build_ui, dump_matches};
use niri::{spawn_event_thread, Win};
use style::apply_css;
fn main() -> glib::ExitCode {
    if env::args().any(|a| a == "--settings") {
        return settings::run();
    }

    if env::args().any(|a| a == "--dump") {
        dump_matches();
        return glib::ExitCode::SUCCESS;
    }

    let app = gtk::Application::builder()
        .application_id("io.github.ricarsz.niridock")
        .build();

    app.connect_startup(|_| {
        apply_css(&load_config());
    });

    app.connect_activate(move |app| {
        let entries = Rc::new(load_entries());
        let windows: Arc<Mutex<HashMap<i64, Win>>> = Arc::new(Mutex::new(HashMap::new()));
        let epoch = Arc::new(AtomicU64::new(0));
        spawn_event_thread(windows.clone(), epoch.clone());

        let mut cfg = load_config();
        apply_css(&cfg);
        cfg = load_config();
        let (win, hot) = build_ui(app, cfg, entries.clone(), windows.clone(), epoch.clone());
        let slot: Rc<RefCell<Option<(gtk::ApplicationWindow, Option<gtk::ApplicationWindow>)>>> =
            Rc::new(RefCell::new(Some((win, hot))));
        let last = Rc::new(RefCell::new(config_mtime()));

        let app2 = app.clone();
        let slot2 = slot.clone();
        glib::timeout_add_local(Duration::from_millis(600), move || {
            let now = config_mtime();
            if now != *last.borrow() {
                *last.borrow_mut() = now;
                let cfg = load_config();
                apply_css(&cfg);
                let fresh = build_ui(&app2, cfg, entries.clone(), windows.clone(), epoch.clone());
                let old = slot2.borrow_mut().replace(fresh);
                if let Some((o, h)) = old {
                    o.close();
                    if let Some(h) = h {
                        h.close();
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    });

    app.run()
}

