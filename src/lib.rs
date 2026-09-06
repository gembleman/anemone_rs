#![deny(unsafe_op_in_unsafe_fn)]

mod app;
#[cfg(feature = "benchmark")]
#[path = "../benchmark/mem.rs"]
pub mod bench_mem;
#[cfg(feature = "benchmark")]
pub use app::translation_cache::TranslationCacheStore as BenchmarkTranslationCacheStore;
#[cfg(feature = "benchmark")]
pub use translation::BenchmarkCacheKey;
mod cli;
mod clipboard;
mod config;
#[cfg(feature = "benchmark")]
pub use config::Config as BenchmarkConfig;
#[cfg(feature = "benchmark")]
pub use config::CustomApiConfig as BenchmarkCustomApiConfig;
#[cfg(feature = "benchmark")]
pub use config::EzTransPostprocessEntry as BenchmarkEzTransPostprocessEntry;
mod d2d;
mod dialogs;
mod dpi;
// file_trans와 translation은 benchmark 하네스만 crate 밖에서 쓴다. 평소에는
// pub(crate)로 닫아 둬야 모듈 안의 죽은 코드가 dead_code 경고로 드러난다.
#[cfg(feature = "benchmark")]
pub mod file_trans;
#[cfg(not(feature = "benchmark"))]
pub(crate) mod file_trans;
mod fs_util;
mod hook;
mod hotkey;
mod logging;
mod magnetic;
mod menu;
mod runtime;
#[cfg(feature = "benchmark")]
pub mod translation;
#[cfg(not(feature = "benchmark"))]
pub(crate) mod translation;
mod tray;
mod update;
mod win32;
mod window;

use app::App;

/// DLL 검색을 System32와 명시적 사용자 경로로 제한해 side-loading을 막는다.
fn harden_dll_search_path() {
    use windows_sys::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_SYSTEM32, LOAD_LIBRARY_SEARCH_USER_DIRS, SetDefaultDllDirectories,
    };
    // SAFETY: SetDefaultDllDirectories 는 부수효과 없는 kernel32 호출이며
    // 두 플래그 조합은 Windows 10 에서 항상 유효하다.
    unsafe {
        if SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_USER_DIRS)
            == 0
        {
            // 실패해도 치명적이지 않으므로 경고만 남기고 진행한다.
            eprintln!("SetDefaultDllDirectories failed");
        }
    }
}

/// 모든 shell/COM 기능의 전제로 UI thread를 STA로 한 번 초기화한다.
fn init_com_sta() {
    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
    };
    // SAFETY: 메인 스레드에서 가장 먼저 한 번만 호출한다. 이미 다른 모드로
    // 초기화되어 있다면 RPC_E_CHANGED_MODE 가 반환되지만, 그래도 무시한다.
    unsafe {
        let hr = CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        );
        if hr < 0 {
            tracing::warn!("CoInitializeEx returned HRESULT 0x{hr:08X}");
        }
    }
}

/// CLI 또는 GUI 애플리케이션을 실행한다.
pub fn run() {
    // 가장 먼저 DLL 검색 경로를 잠근다 (다른 의존성 초기화 전에).
    harden_dll_search_path();

    if let Err(error) = runtime::initialize() {
        eprintln!("Anemone 데이터 경로 초기화 실패: {error}");
        std::process::exit(1);
    }

    // CLI는 GUI, tracing, COM, D2D 초기화 없이 처리한다.
    match cli::run() {
        cli::CliOutcome::Done(code) => {
            std::process::exit(code);
        }
        cli::CliOutcome::Gui => {}
    }

    if let Err(error) = logging::init(config::Config::peek_hook_debug_log()) {
        logging::report_init_failure(error.as_ref());
    }

    // GUI 시작 경로에서만, 프로세스당 한 번 호출한다. `runtime::initialize()`에
    // 넣으면 안 되는 이유는 `update::apply::cleanup_backup`의 문서를 참고 —
    // EzTrans helper 서브커맨드(`cli::run`이 위에서 이미 처리했다)도 그 경로를
    // 타면 helper가 뜰 때마다 삭제를 시도하게 된다.
    //
    // logging::init() 뒤여야 한다. 이 함수의 로그는 업데이트가 실제로 적용됐는지
    // 알려주는 유일한 단서인데, subscriber 설치 전에 부르면 통째로 버려진다.
    update::apply::cleanup_backup();

    // UI 스레드 COM(STA) 1회 초기화 — 모든 다이얼로그/셸 호출의 공통 전제.
    init_com_sta();

    if let Err(e) = App::run() {
        tracing::error!("Error: {e}");
        let message: Vec<u16> = format!("아네모네를 시작할 수 없습니다.\n\n{e}")
            .encode_utf16()
            .chain([0])
            .collect();
        let caption: Vec<u16> = "아네모네 오류".encode_utf16().chain([0]).collect();
        unsafe {
            let _ = windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                caption.as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
        // 시작 실패의 원인을 적은 줄이 아직 대기열에 있다. 여기서 내보내지
        // 않으면 정작 볼 것이 없는 로그만 남는다.
        logging::flush_and_stop();
        std::process::exit(1);
    }

    // App::run()이 반환한 시점에는 AppCleanupGuard::drop이 이미 config를 저장하고
    // AppServices::shutdown()까지 끝냈다. 업데이트 적용 중 재시작이 요청됐다면
    // 그 뒤에야 새 exe를 띄워, 두 프로세스가 config.toml을 동시에 만지지 않게 한다.
    app::restart_after_update_if_requested();

    // 대기열에 남은 줄을 파일에 마저 쓴다. 재시작 요청이 있었다면 새 프로세스가
    // 이미 떴지만, 로그 파일은 append라 서로 덮어쓰지 않는다.
    logging::flush_and_stop();
}
