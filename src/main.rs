#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod claude;
mod config;
mod cursor;
mod model;
mod taskbar;
mod ui;

use model::{ProviderSnapshot, ProviderStatus};

fn print_snapshot(s: &ProviderSnapshot) {
    let plan = s
        .plan
        .as_deref()
        .map(|p| format!(" [{p}]"))
        .unwrap_or_default();

    let status_line = match &s.status {
        ProviderStatus::Ok => "Ok".to_string(),
        ProviderStatus::NotSignedIn(msg)
        | ProviderStatus::Error(msg)
        | ProviderStatus::Stale(msg) => msg.clone(),
    };

    println!("{}{plan} {status_line}", s.name);

    if s.status.has_meters() {
        for m in &s.meters {
            let mut line = format!("  {:6} {:4.1}%", m.label, m.percent);
            if let Some(resets) = m.resets_at {
                let countdown = model::format_countdown(resets, s.fetched_at);
                line.push_str(&format!("  resets in {countdown}"));
            }
            if let Some(ref detail) = m.detail {
                line.push_str(&format!("  {detail}"));
            }
            println!("{line}");
        }
    }
}

fn print_once(cfg: &config::Config) {
    if let Some(path) = config::config_path() {
        println!("Config: {}", path.display());
    }
    print_snapshot(&claude::fetch());
    print_snapshot(&cursor::fetch(cfg));
}

fn main() {
    let _ = config::ensure_exists();
    let cfg = config::load();

    let once = std::env::args().any(|a| a == "--once");
    if once {
        print_once(&cfg);
        std::process::exit(0);
    }

    if let Err(e) = ui::run(cfg) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
