//! GUI, tracing, COM, D2D 초기화 없이 번역을 수행하는 headless CLI.

mod file_trans;
mod list;
mod translate;

use std::ffi::OsString;

use clap::{Parser, Subcommand, ValueEnum};

use crate::translation::TranslationEngine;

const AFTER_HELP: &str = r#"인자 없이 실행하면 GUI 모드로 시작합니다.

ENGINE: eztrans | google | deepl | papago | llm | custom
LANG:   ISO 639-1 (예: ja, ko, en, zh)"#;

#[derive(Parser)]
#[command(
    name = "anemone_rs",
    about = "Windows 오버레이 번역 도구",
    after_help = AFTER_HELP
)]
struct Cli {
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
    /// 내부 EzTrans helper 프로세스. 직접 호출하지 않는다.
    #[command(hide = true)]
    EztransWorker {
        #[arg(long)]
        dll: String,
        #[arg(long)]
        dat: String,
    },
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
    #[value(name = "custom")]
    Custom,
}

impl From<Engine> for TranslationEngine {
    fn from(value: Engine) -> Self {
        match value {
            Engine::EzTrans => Self::EzTrans,
            Engine::Google => Self::Google,
            Engine::DeepL => Self::DeepL,
            Engine::Papago => Self::Papago,
            Engine::Llm => Self::Llm,
            Engine::Custom => Self::Custom,
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

/// 결과를 표준 stream에 쓰고 종료 여부와 code를 반환한다.
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
        Command::Translate(args) => translate::run(args),
        Command::FileTrans(args) => file_trans::run(args),
        Command::ListEngines => list::engines(),
        Command::ListLangs(args) => list::languages(args),
        Command::EztransWorker { dll, dat } => crate::translation::run_eztrans_worker(&dll, &dat),
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
