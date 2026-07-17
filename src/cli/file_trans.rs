use std::path::PathBuf;
use std::sync::Arc;

use clap::ValueEnum;

use crate::config::Config;
use crate::translation::http_common::shared_client;
use crate::translation::worker::{TranslationDispatch, TranslationRequest};
use crate::translation::{TranslationEngine, get_eztrans_manager};

use super::helpers::{JsonVal, json_object};
use super::translate::{build_credentials, resolve_engine, resolve_languages};

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

pub(super) fn run(args: Args, json: bool) -> Result<(), String> {
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
    let engine = resolve_engine(engine, &config);
    let (source_lang, target_lang) = resolve_languages(&source, &target, &config, engine)?;

    // EzTrans 사전 초기화 — 라인마다 같은 에러로 실패하는 것보다 사전 차단.
    if engine == TranslationEngine::EzTrans {
        let defaults = crate::config::TranslationConfig::default();
        let dll = if config.translation.eztrans_dll_path.is_empty() {
            defaults.eztrans_dll_path.clone()
        } else {
            config.translation.eztrans_dll_path.clone()
        };
        let dat = if config.translation.eztrans_dat_path.is_empty() {
            defaults.eztrans_dat_path.clone()
        } else {
            config.translation.eztrans_dat_path.clone()
        };
        let manager = get_eztrans_manager();
        let mut mgr = manager
            .lock()
            .map_err(|_| "EzTrans 매니저 잠금 실패".to_string())?;
        mgr.init(&dll, &dat)
            .map_err(|e| format!("EzTrans 초기화 실패: {e}"))?;
    }

    let credentials = build_credentials(engine, &config)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio 런타임 생성 실패: {e}"))?;
    let client = shared_client();

    use std::fs::File;
    use std::io::{BufRead, BufReader, BufWriter, Write};

    let body = crate::util::read_utf8_translation_input(&input)?;
    let reader = BufReader::new(std::io::Cursor::new(body));

    let out_file = File::create(&output).map_err(|e| {
        format!(
            "출력 파일을 생성할 수 없습니다: {} ({})",
            output.display(),
            e
        )
    })?;
    let mut writer = BufWriter::new(out_file);
    writer
        .write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|e| format!("BOM 쓰기 실패: {e}"))?;

    let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();
    let total = lines.len();
    let last_idx = total.saturating_sub(1);

    for (i, line) in lines.iter().enumerate() {
        let translated = if line.is_empty() || (no_trans_linefeed && line.trim().is_empty()) {
            line.clone()
        } else {
            let req = TranslationRequest {
                id: 0,
                text: Arc::from(line.as_str()),
                engine,
                source_lang,
                target_lang,
                credentials: credentials.clone(),
            };
            match rt.block_on(TranslationDispatch::translate_async(&req, &client)) {
                Ok(s) => s,
                Err(e) => format!("[번역 실패: {e}]"),
            }
        };
        write_line(&mut writer, line, &translated, write_type, i == last_idx)?;
    }
    writer.flush().map_err(|e| format!("flush 실패: {e}"))?;

    if json {
        println!(
            "{}",
            json_object(&[
                ("input", JsonVal::Str(&input.to_string_lossy())),
                ("output", JsonVal::Str(&output.to_string_lossy())),
                ("lines", JsonVal::Num(total as i64)),
                ("engine", JsonVal::Str(engine.to_str())),
            ])
        );
    } else {
        println!(
            "완료: {} → {} ({} 줄)",
            input.display(),
            output.display(),
            total
        );
    }
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

fn write_line(
    writer: &mut std::io::BufWriter<std::fs::File>,
    original: &str,
    translated: &str,
    write_type: WriteType,
    is_last: bool,
) -> Result<(), String> {
    use std::io::Write;
    let mut io = |buf: &str| writeln!(writer, "{buf}").map_err(|e| e.to_string());
    match write_type {
        WriteType::Only => io(translated)?,
        WriteType::Both => {
            io(original)?;
            if is_last {
                write!(writer, "{translated}").map_err(|e| e.to_string())?;
            } else {
                io(translated)?;
            }
        }
        WriteType::BothNl => {
            io(original)?;
            io(translated)?;
            if !is_last {
                writeln!(writer).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}
