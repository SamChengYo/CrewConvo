//! CrewConvo Personal as a Windows desktop app.
//!
//! One program: it runs the server and its built-in database, shows the web interface in its own
//! window, sets up the coding agents on this computer and keeps them connected from the system
//! tray after the window is closed.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod update;

fn main() {
    #[cfg(windows)]
    app::run();
    #[cfg(not(windows))]
    eprintln!("The CrewConvo desktop app is for Windows. On this system run crewconvo-server instead.");
}
