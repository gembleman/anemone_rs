// 진단 중 — stdout 로그 확인용으로 console 서브시스템 유지.
// #![windows_subsystem = "windows"]

mod app;
mod bench;
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
use config::Config;
use translation::{EzTransTranslator, Language};

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

/// 명령행 인자 파싱 결과.
enum CliMode {
    /// 인자 없음 → 기존 GUI 모드 실행.
    Gui,
    /// `--translate <text>` → eztrans로 번역 후 결과만 stdout, 종료.
    Translate(String),
    /// `--help` / `-h` → 사용법 출력, 정상 종료.
    Help,
}

fn parse_cli() -> Result<CliMode, String> {
    let mut args = std::env::args().skip(1);
    let Some(first) = args.next() else {
        return Ok(CliMode::Gui);
    };

    match first.as_str() {
        "-h" | "--help" => Ok(CliMode::Help),
        "--translate" => match args.next() {
            Some(text) => Ok(CliMode::Translate(text)),
            None => Err("--translate 뒤에 번역할 텍스트가 필요합니다.".to_string()),
        },
        other if other.starts_with("--translate=") => {
            Ok(CliMode::Translate(other["--translate=".len()..].to_string()))
        }
        other => Err(format!("알 수 없는 인자: {other}")),
    }
}

fn print_usage() {
    println!("anemone_rs — Windows 오버레이 번역 도구");
    println!();
    println!("USAGE:");
    println!("    anemone_rs.exe                       GUI 모드로 실행 (기본)");
    println!("    anemone_rs.exe --translate <TEXT>    EzTrans로 번역 후 결과만 출력");
    println!("    anemone_rs.exe -h, --help            이 도움말 출력");
    println!();
    println!("NOTE:");
    println!("    --translate는 config.toml의 eztrans_dll_path/eztrans_dat_path를 사용합니다.");
    println!("    config.toml은 실행 파일과 같은 폴더에서 읽거나 생성됩니다.");
}

/// CLI 번역 모드. eztrans만 사용(JP→KR). 성공 시 결과를 stdout에 출력.
fn run_translate_cli(text: &str) -> Result<(), String> {
    let config = Config::load_or_default();
    let defaults = config::TranslationConfig::default();
    // 기존 config.toml에 빈 값이 저장된 환경을 위해, 비어 있으면 기본값(=exe 옆 eztrans_dll)으로 폴백.
    let dll = if config.translation.eztrans_dll_path.is_empty() {
        defaults.eztrans_dll_path.as_str()
    } else {
        config.translation.eztrans_dll_path.as_str()
    };
    let dat = if config.translation.eztrans_dat_path.is_empty() {
        defaults.eztrans_dat_path.as_str()
    } else {
        config.translation.eztrans_dat_path.as_str()
    };

    if dll.is_empty() || dat.is_empty() {
        return Err(
            "eztrans 경로를 확인할 수 없습니다. config.toml의 eztrans_dll_path/eztrans_dat_path를 설정하세요."
                .to_string(),
        );
    }

    let translator = EzTransTranslator::new(dll, dat)
        .map_err(|e| format!("EzTrans 초기화 실패: {e}"))?;

    let translated = translator
        .translate(text, Language::Jpn, Language::Kor)
        .map_err(|e| format!("번역 실패: {e}"))?;

    println!("{translated}");
    Ok(())
}

fn main() {
    // 가장 먼저 DLL 검색 경로를 잠근다 (다른 의존성 초기화 전에).
    harden_dll_search_path();

    let mode = match parse_cli() {
        Ok(m) => m,
        Err(msg) => {
            eprintln!("{msg}");
            print_usage();
            std::process::exit(2);
        }
    };

    match mode {
        CliMode::Help => {
            print_usage();
            return;
        }
        CliMode::Translate(text) => {
            // CLI 모드: tracing/COM/GUI 초기화 생략. 결과만 stdout.
            if let Err(e) = run_translate_cli(&text) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        CliMode::Gui => {}
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
