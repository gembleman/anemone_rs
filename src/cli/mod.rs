//! CLI(headless) 진입점.
//!
//! GUI 다이얼로그 없이 번역/설정/파일 번역/메타 조회를 수행할 수 있도록 한다.
//! 자동화·AI 테스트 친화성을 우선해 사람이 읽기 좋은 plain text 를 기본으로
//! 내고, `--json` 플래그가 있으면 구조화 출력으로 전환한다.
//!
//! Win32/GUI 초기화 경로는 거치지 않으므로 console 서브시스템에서 그대로
//! 동작하며, `tracing`/COM/D2D 초기화도 생략한다.

mod config;
mod file_trans;
mod helpers;
mod list;
mod translate;
mod usage;

pub use usage::print_usage;

/// CLI 실행 결과. `main` 의 종료 코드와 매핑된다.
pub enum CliOutcome {
    /// CLI 처리를 완료하고 종료해야 함. exit code 포함.
    Done(i32),
    /// CLI 인자가 없어 GUI 로 진입해야 함.
    Gui,
}

/// CLI 진입점.
///
/// `std::env::args` 를 직접 파싱해 의존성을 추가하지 않는다. 결과 출력은
/// stdout/stderr 로 흘려보내고, 종료 코드는 `CliOutcome::Done(code)` 로 알린다.
pub fn run() -> CliOutcome {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        return CliOutcome::Gui;
    }

    // 전역 플래그(`--json`) 를 먼저 분리한다. 위치는 어디든 허용.
    let mut json = false;
    let mut rest: Vec<String> = Vec::with_capacity(argv.len());
    for a in argv {
        if a == "--json" {
            json = true;
        } else {
            rest.push(a);
        }
    }

    let cmd = rest.remove(0);
    let args = rest;

    let result: Result<(), String> = match cmd.as_str() {
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        "translate" => translate::run(&args, json),
        "file-trans" => file_trans::run(&args, json),
        "list-engines" => list::engines(json),
        "list-langs" => list::languages(&args, json),
        "config" => config::run(&args, json),
        "config-path" => config::print_path(json),
        other => Err(format!("알 수 없는 명령: {other}")),
    };

    match result {
        Ok(()) => CliOutcome::Done(0),
        Err(msg) => {
            eprintln!("error: {msg}");
            CliOutcome::Done(1)
        }
    }
}
