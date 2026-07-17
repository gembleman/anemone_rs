// 진단 중 — stdout 로그 확인용으로 console 서브시스템 유지.
// #![windows_subsystem = "windows"]

mod app;
mod bench;
mod cli;
mod clipboard;
mod config;
mod constants;
mod d2d;
mod d2d_composition;
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
        if let Err(e) =
            SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_USER_DIRS)
        {
            // 실패해도 치명적이지 않으므로 경고만 남기고 진행한다.
            eprintln!("SetDefaultDllDirectories failed: {e}");
        }
    }
}

/// UI 스레드를 STA 로 한 번만 초기화한다.
///
/// 파일 다이얼로그(Common Item Dialog), 작업표시줄 진행률(ITaskbarList3),
/// 셸 아이템 등 모든 COM 사용 경로의 공통 전제. 다이얼로그마다 init/uninit
/// 짝짓는 모델은 다른 COM 객체의 수명을 무너뜨릴 수 있어 사용하지 않는다.
/// 프로세스 종료 시 OS 가 정리하므로 명시적 `CoUninitialize` 는 생략한다.
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

    // CLI 인자가 있으면 headless 경로로 처리하고 종료. GUI 초기화(tracing/COM/D2D)는
    // 거치지 않는다 — 콘솔에서 자동화/AI 테스트가 가벼워야 하기 때문.
    match cli::run() {
        cli::CliOutcome::Done(code) => {
            std::process::exit(code);
        }
        cli::CliOutcome::Gui => {}
    }

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(false)
        .init();

    // UI 스레드 COM(STA) 1회 초기화 — 모든 다이얼로그/셸 호출의 공통 전제.
    init_com_sta();

    if let Err(e) = App::run() {
        tracing::error!("Error: {e}");
    }
}
