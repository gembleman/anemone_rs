#![windows_subsystem = "windows"]

mod app;
mod clipboard;
mod config;
mod d2d;
mod dialogs;
mod hotkey;
mod magnetic;
mod menu;
mod screenshot;
mod translation;
mod tray;
mod window;

use app::App;

fn main() {
    if let Err(e) = App::run() {
        eprintln!("Error: {e}");
    }
}
