use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clap::ValueEnum;

use crate::config::Config;
use crate::file_trans::{
    FileTranslationProgress, FileTranslationRequest, WriteType as CoreWriteType,
    run as run_file_trans,
};
use crate::translation::PreparedJob;

use super::translate::{resolve_engine, resolve_languages};

#[derive(clap::Args)]
pub(super) struct Args {
    /// 입력 파일
    #[arg(long = "in")]
    input: PathBuf,
    /// 출력 파일
    #[arg(long = "out")]
    output: PathBuf,
    /// 사용할 번역 엔진
    #[arg(long, value_enum)]
    engine: Option<super::Engine>,
    /// 소스 언어 코드 (예: ja)
    #[arg(long = "from")]
    source: Option<String>,
    /// 타겟 언어 코드 (예: ko)
    #[arg(long = "to")]
    target: Option<String>,
    /// 출력 형식
    #[arg(long, value_enum, default_value = "only")]
    format: WriteType,
    /// 공백 줄을 번역하지 않음
    #[arg(long)]
    no_trans_linefeed: bool,
}

pub(super) fn run(args: Args) -> Result<(), String> {
    run_with_config(args, Config::load_or_default())
}

/// `run`에서 설정 로드를 분리해, 테스트가 실제 사용자 설정 파일을 건드리지
/// 않고 파일 번역 실행 경로(엔진/언어 결정, 실제 파일 I/O와 번역 호출)를
/// 검증할 수 있게 한다.
pub(super) fn run_with_config(args: Args, config: Config) -> Result<(), String> {
    let Args {
        input,
        output,
        engine,
        source,
        target,
        format: write_type,
        no_trans_linefeed,
    } = args;

    let engine = resolve_engine(engine, &config)?;
    let (source_lang, target_lang) = resolve_languages(&source, &target, &config)?;

    let translation =
        PreparedJob::with_engine_languages(&config.translation, engine, source_lang, target_lang)
            .map_err(|error| error.to_string())?;
    let job = FileTranslationRequest {
        input_files: vec![input],
        output_files: vec![output],
        write_type: write_type.into(),
        no_trans_linefeed,
        cancel_token: Arc::new(AtomicBool::new(false)),
        translation,
    };
    let total = Cell::new(0usize);
    let error = RefCell::new(None);

    run_file_trans(&job, |event| match event {
        FileTranslationProgress::TotalLines(value) => total.set(value.max(0) as usize),
        FileTranslationProgress::Finished(Err(message)) => {
            *error.borrow_mut() = Some(message.to_string())
        }
        _ => {}
    });

    if let Some(error) = error.into_inner() {
        return Err(error);
    }

    let input = &job.input_files[0];
    let output = &job.output_files[0];
    let total = total.get();

    println!(
        "완료: {} → {} ({} 줄)",
        input.display(),
        output.display(),
        total
    );
    Ok(())
}

#[derive(Clone, Copy, ValueEnum)]
enum WriteType {
    /// 번역만
    Only,
    /// 원문 + 번역
    Both,
    /// 원문 + 번역 + 빈 줄
    BothNl,
}

impl From<WriteType> for CoreWriteType {
    fn from(value: WriteType) -> Self {
        match value {
            WriteType::Only => Self::TranslationOnly,
            WriteType::Both => Self::OriginalAndTrans,
            WriteType::BothNl => Self::OriginalTransNewline,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/cli/file_trans.rs"]
mod tests;
