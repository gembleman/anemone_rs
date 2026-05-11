#![windows_subsystem = "windows"]

mod app;
mod clipboard;
mod config;
mod constants;
mod d2d;
mod dialogs;
mod dpi;
mod hotkey;
mod magnetic;
mod menu;
mod translation;
mod tray;
mod util;
mod window;

use app::App;

/// DLL 검색 경로를 System32 와 사용자 추가 경로(`AddDllDirectory`)로 제한해
/// cwd/PATH 의 동명 DLL 사이드로드를 차단한다. EzTrans DLL 처럼 사용자가
/// 지정한 절대 경로로 로드하는 경우에는 영향이 없다.
fn harden_dll_search_path() {
    use windows::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_SYSTEM32, LOAD_LIBRARY_SEARCH_USER_DIRS, SetDefaultDllDirectories,
    };
    // SAFETY: SetDefaultDllDirectories 는 부수효과 없는 kernel32 호출이며
    // 두 플래그 조합은 Windows 10 에서 항상 유효하다.
    unsafe {
        if let Err(e) = SetDefaultDllDirectories(
            LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_USER_DIRS,
        ) {
            // 실패해도 치명적이지 않으므로 경고만 남기고 진행한다.
            eprintln!("SetDefaultDllDirectories failed: {e}");
        }
    }
}

fn main() {
    // 가장 먼저 DLL 검색 경로를 잠근다 (다른 의존성 초기화 전에).
    harden_dll_search_path();

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(false)
        .init();

    if let Err(e) = App::run() {
        tracing::error!("Error: {e}");
    }
}
