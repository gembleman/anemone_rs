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

use std::ffi::OsString;

use clap::{Parser, Subcommand, ValueEnum};

use crate::translation::TranslationEngine;

const AFTER_HELP: &str = "인자 없이 실행하면 GUI 모드로 시작합니다.\n\
\n\
ENGINE: eztrans | google | deepl | papago | llm\n\
LANG:   ISO 639-1 (예: ja, ko, en, zh)\n\
\n\
CONFIG KEYS (대표):\n\
    translation.engine, translation.source_lang, translation.target_lang\n\
    translation.eztrans_dll_path, translation.eztrans_dat_path\n\
    translation.deepl_api_key, translation.papago_client_id\n\
    translation.papago_client_secret\n\
    translation.llm.provider, translation.llm.model, translation.llm.api_key\n\
    translation.llm.base_url, translation.llm.temperature, translation.llm.max_tokens\n\
    clipboard_watch, click_through, magnetic_mode, background_visible\n\
    border_visible, window_topmost, window_visible";

#[derive(Parser)]
#[command(
    name = "anemone_rs",
    about = "Windows 오버레이 번역 도구",
    after_help = AFTER_HELP
)]
struct Cli {
    /// 결과를 JSON 한 줄로 출력
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 텍스트 번역
    Translate(translate::Args),
    /// 파일을 한 줄씩 번역해 출력 파일에 기록
    FileTrans(file_trans::Args),
    /// 지원하는 번역 엔진 출력
    ListEngines,
    /// 엔진이 지원하는 언어 출력
    ListLangs(list::LanguagesArgs),
    /// config.toml 조회 및 변경
    Config {
        #[command(subcommand)]
        command: config::Command,
    },
    /// config.toml 절대 경로 출력
    ConfigPath,
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum Engine {
    #[value(name = "eztrans")]
    EzTrans,
    #[value(name = "google")]
    Google,
    #[value(name = "deepl")]
    DeepL,
    #[value(name = "papago")]
    Papago,
    #[value(name = "llm")]
    Llm,
}

impl From<Engine> for TranslationEngine {
    fn from(value: Engine) -> Self {
        match value {
            Engine::EzTrans => Self::EzTrans,
            Engine::Google => Self::Google,
            Engine::DeepL => Self::DeepL,
            Engine::Papago => Self::Papago,
            Engine::Llm => Self::Llm,
        }
    }
}

/// CLI 실행 결과. `main` 의 종료 코드와 매핑된다.
pub enum CliOutcome {
    /// CLI 처리를 완료하고 종료해야 함. exit code 포함.
    Done(i32),
    /// CLI 인자가 없어 GUI 로 진입해야 함.
    Gui,
}

/// CLI 진입점.
///
/// 결과 출력은 stdout/stderr 로 흘려보내고, 종료 코드는
/// `CliOutcome::Done(code)` 로 알린다.
pub fn run() -> CliOutcome {
    let argv: Vec<OsString> = std::env::args_os().collect();
    if argv.len() <= 1 {
        return CliOutcome::Gui;
    }

    let cli = match Cli::try_parse_from(argv) {
        Ok(cli) => cli,
        Err(error) => {
            let code = error.exit_code();
            let _ = error.print();
            return CliOutcome::Done(code);
        }
    };

    let result = match cli.command {
        Command::Translate(args) => translate::run(args, cli.json),
        Command::FileTrans(args) => file_trans::run(args, cli.json),
        Command::ListEngines => list::engines(cli.json),
        Command::ListLangs(args) => list::languages(args, cli.json),
        Command::Config { command } => config::run(command, cli.json),
        Command::ConfigPath => config::print_path(cli.json),
    };

    match result {
        Ok(()) => CliOutcome::Done(0),
        Err(msg) => {
            eprintln!("error: {msg}");
            CliOutcome::Done(1)
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/cli/mod.rs"]
mod tests;
