#![windows_subsystem = "windows"]

mod window;

use window::App;

fn main() {
    if let Err(e) = App::run() {
        eprintln!("Error: {e}");
    }
}
