#![windows_subsystem = "windows"]

mod app;
mod config;
mod dialogs;
mod tray;
mod menu;
mod hotkey;
mod clipboard;
mod window;
mod magnetic;
mod screenshot;
mod file_watch;

use app::App;

fn main() {
    if let Err(e) = App::run() {
        eprintln!("Error: {e}");
    }
}
