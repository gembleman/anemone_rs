use std::path::PathBuf;
use std::sync::Arc;

use crate::config::Config;
use crate::translation::http_common::shared_client;
use crate::translation::worker::{TranslationDispatch, TranslationRequest};
use crate::translation::{TranslationEngine, get_eztrans_manager};

use super::helpers::{JsonVal, get_value, json_object};
use super::translate::{build_credentials, resolve_engine, resolve_languages};

pub(super) fn run(args: &[String], json: bool) -> Result<(), String> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut engine_override: Option<String> = None;
    let mut from_override: Option<String> = None;
    let mut to_override: Option<String> = None;
    let mut format_str: Option<String> = None;
    let mut no_trans_linefeed = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--in" => input = Some(PathBuf::from(get_value(args, &mut i, "--in")?)),
            "--out" => output = Some(PathBuf::from(get_value(args, &mut i, "--out")?)),
            "--engine" => engine_override = Some(get_value(args, &mut i, "--engine")?),
            "--from" => from_override = Some(get_value(args, &mut i, "--from")?),
            "--to" => to_override = Some(get_value(args, &mut i, "--to")?),
            "--format" => format_str = Some(get_value(args, &mut i, "--format")?),
            "--no-trans-linefeed" => {
                no_trans_linefeed = true;
                i += 1;
            }
            a => return Err(format!("알 수 없는 옵션: {a}")),
        }
    }

    let input = input.ok_or_else(|| "--in <FILE> 이 필요합니다.".to_string())?;
    let output = output.ok_or_else(|| "--out <FILE> 이 필요합니다.".to_string())?;
    let write_type = parse_write_type(format_str.as_deref())?;

    let config = Config::load_or_default();
    let engine = resolve_engine(&engine_override, &config)?;
    let (source_lang, target_lang) =
        resolve_languages(&from_override, &to_override, &config, engine)?;

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

#[derive(Clone, Copy)]
enum WriteType {
    /// 번역만
    Only,
    /// 원문 + 번역
    Both,
    /// 원문 + 번역 + 빈 줄
    BothNl,
}

fn parse_write_type(s: Option<&str>) -> Result<WriteType, String> {
    match s.unwrap_or("only") {
        "only" => Ok(WriteType::Only),
        "both" => Ok(WriteType::Both),
        "both-nl" => Ok(WriteType::BothNl),
        other => Err(format!(
            "알 수 없는 --format: {other} (only|both|both-nl 중 하나)"
        )),
    }
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
