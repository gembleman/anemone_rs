#![windows_subsystem = "windows"]

mod app;
mod clipboard;
mod config;
mod d2d;
mod dialogs;
mod hotkey;
mod magnetic;
mod menu;
mod translation;
mod tray;
mod window;

use app::App;

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(false)
        .init();

    if let Err(e) = App::run() {
        tracing::error!("Error: {e}");
    }
}
