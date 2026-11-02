//! Crash diagnostics for builds that run without a console.
//!
//! A Windows release build is compiled with `windows_subsystem = "windows"`, so
//! a panic message would otherwise disappear with the window. The hook
//! installed here writes the panic, its location, a backtrace and the recent
//! diagnostic trail to `%APPDATA%\StarFile\panic.log`, and the trail says
//! which palette transition the app was in when it died.

use std::path::PathBuf;
use std::sync::Mutex;

/// How many diagnostic events are kept for a crash report.
const TRAIL: usize = 64;

static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Records one diagnostic event (kept in memory, dumped on panic).
pub fn log(event: impl AsRef<str>) {
    if let Ok(mut events) = EVENTS.lock() {
        if events.len() == TRAIL {
            events.remove(0);
        }
        events.push(event.as_ref().to_string());
    }
}

/// `%APPDATA%\StarFile\panic.log`, or the temp directory as a fallback.
pub fn log_path() -> PathBuf {
    dirs::config_dir()
        .map(|dir| dir.join("StarFile").join("panic.log"))
        .unwrap_or_else(|| std::env::temp_dir().join("starfile-panic.log"))
}

/// Installs the panic hook. Call once, before any window is created.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut report = String::new();
        report.push_str(&format!(
            "starfile {} ({}) at {}\n",
            env!("CARGO_PKG_VERSION"),
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        ));
        if let Some(location) = info.location() {
            report.push_str(&format!("at {}:{}\n", location.file(), location.line()));
        }
        report.push_str(&format!("{info}\n\nrecent events:\n"));
        if let Ok(events) = EVENTS.lock() {
            for event in events.iter() {
                report.push_str("  ");
                report.push_str(event);
                report.push('\n');
            }
        }
        report.push_str("\nbacktrace:\n");
        report.push_str(&std::backtrace::Backtrace::force_capture().to_string());
        let _ = std::fs::write(log_path(), &report);
        eprintln!("{report}");
        previous(info);
    }));
}
