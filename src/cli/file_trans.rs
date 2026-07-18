use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clap::ValueEnum;

use crate::config::Config;
use crate::file_trans::{
    FileTransJobData, ProgressEvent, WriteType as CoreWriteType, run as run_file_trans,
};
use crate::translation::TranslationJobSpec;

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
    let Args {
        input,
        output,
        engine,
        source,
        target,
        format: write_type,
        no_trans_linefeed,
    } = args;

    let config = Config::load_or_default();
    let engine = resolve_engine(engine, &config)?;
    let (source_lang, target_lang) = resolve_languages(&source, &target, &config)?;

    let spec = TranslationJobSpec::with_engine_languages(
        &config.translation,
        engine,
        source_lang,
        target_lang,
    )
    .map_err(|error| error.to_string())?;
    let (engine, source_lang, target_lang, credentials, eztrans_process) = spec.into_file_parts();

    let job = FileTransJobData {
        input_files: vec![input],
        output_files: vec![output],
        write_type: write_type.into(),
        no_trans_linefeed,
        cancel_token: Arc::new(AtomicBool::new(false)),
        engine,
        source_lang,
        target_lang,
        credentials,
        eztrans_process,
    };
    let total = Cell::new(0usize);
    let error = RefCell::new(None);

    run_file_trans(&job, |event| match event {
        ProgressEvent::TotalLines(value) => total.set(value.max(0) as usize),
        ProgressEvent::Error(message) => *error.borrow_mut() = Some(message),
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
