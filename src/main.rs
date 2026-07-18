#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod backlog;
mod cli;
mod clipboard;
mod config;
mod constants;
mod d2d;
mod dialogs;
mod dpi;
mod file_trans;
mod fs_util;
mod hotkey;
mod logging;
mod magnetic;
mod menu;
mod runtime;
mod settings_model;
mod translation;
mod translation_ui;
mod tray;
mod util;
mod window;

use app::App;

/// DLL 검색을 System32와 명시적 사용자 경로로 제한해 side-loading을 막는다.
fn harden_dll_search_path() {
    use windows::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_SYSTEM32, LOAD_LIBRARY_SEARCH_USER_DIRS, SetDefaultDllDirectories,
    };
    // SAFETY: SetDefaultDllDirectories 는 부수효과 없는 kernel32 호출이며
    // 두 플래그 조합은 Windows 10 에서 항상 유효하다.
    unsafe {
        if let Err(e) =
            SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_USER_DIRS)
        {
            // 실패해도 치명적이지 않으므로 경고만 남기고 진행한다.
            eprintln!("SetDefaultDllDirectories failed: {e}");
        }
    }
}

/// 모든 shell/COM 기능의 전제로 UI thread를 STA로 한 번 초기화한다.
fn init_com_sta() {
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
    };
    // SAFETY: 메인 스레드에서 가장 먼저 한 번만 호출한다. 이미 다른 모드로
    // 초기화되어 있다면 RPC_E_CHANGED_MODE 가 반환되지만, 그래도 무시한다.
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        if hr.is_err() {
            tracing::warn!("CoInitializeEx returned {hr:?}");
        }
    }
}

fn main() {
    // 가장 먼저 DLL 검색 경로를 잠근다 (다른 의존성 초기화 전에).
    harden_dll_search_path();

    // CLI는 GUI, tracing, COM, D2D 초기화 없이 처리한다.
    match cli::run() {
        cli::CliOutcome::Done(code) => {
            std::process::exit(code);
        }
        cli::CliOutcome::Gui => {}
    }

    if let Err(error) = logging::init() {
        logging::report_init_failure(error.as_ref());
    }

    // UI 스레드 COM(STA) 1회 초기화 — 모든 다이얼로그/셸 호출의 공통 전제.
    init_com_sta();

    if let Err(e) = App::run() {
        tracing::error!("Error: {e}");
    }
}
