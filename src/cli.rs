//! CLI(headless) 진입점.
//!
//! GUI 다이얼로그 없이 번역/설정/파일 번역/메타 조회를 수행할 수 있도록 한다.
//! 자동화·AI 테스트 친화성을 우선해 사람이 읽기 좋은 plain text 를 기본으로
//! 내고, `--json` 플래그가 있으면 구조화 출력으로 전환한다.
//!
//! Win32/GUI 초기화 경로는 거치지 않으므로 console 서브시스템에서 그대로
//! 동작하며, `tracing`/COM/D2D 초기화도 생략한다.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::Config;
use crate::translation::http_common::shared_client;
use crate::translation::worker::{TranslationDispatch, TranslationRequest};
use crate::translation::{
    EngineCredentials, Language, TranslationEngine, get_eztrans_manager, lang_utils,
    translate_with_eztrans,
};

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
        "translate" => cmd_translate(&args, json),
        "file-trans" => cmd_file_trans(&args, json),
        "list-engines" => cmd_list_engines(json),
        "list-langs" => cmd_list_langs(&args, json),
        "config" => cmd_config(&args, json),
        "config-path" => cmd_config_path(json),
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

pub fn print_usage() {
    println!("anemone_rs — Windows 오버레이 번역 도구");
    println!();
    println!("USAGE:");
    println!("    anemone_rs.exe                              GUI 모드로 실행 (기본)");
    println!("    anemone_rs.exe <COMMAND> [ARGS...] [--json] 명령 실행");
    println!();
    println!("COMMANDS:");
    println!("    translate <TEXT> [--engine E] [--from L] [--to L] [--stdin]");
    println!(
        "                                  텍스트 번역. config.toml 의 키/엔진을 기본값으로 사용"
    );
    println!("    file-trans --in <FILE> --out <FILE> [--engine E] [--from L] [--to L]");
    println!("               [--format only|both|both-nl] [--no-trans-linefeed]");
    println!("                                  파일을 한 줄씩 번역해 출력 파일에 기록");
    println!("    list-engines                  지원하는 번역 엔진 출력");
    println!("    list-langs [--engine E]       엔진이 지원하는 언어 출력");
    println!("    config show                   현재 config.toml 내용 출력");
    println!(
        "    config get <KEY>              config 값을 점 경로로 조회 (예: translation.engine)"
    );
    println!("    config set <KEY> <VALUE>      config 값 변경 후 저장");
    println!("    config-path                   config.toml 의 절대 경로 출력");
    println!();
    println!("OPTIONS:");
    println!("    --json    결과를 JSON 한 줄로 출력 (기본은 사람이 읽기 좋은 plain text)");
    println!("    -h, --help  이 도움말 출력");
    println!();
    println!("ENGINE: eztrans | google | deepl | papago | llm");
    println!("LANG:   ISO 639-1 (예: ja, ko, en, zh)");
    println!();
    println!("CONFIG KEYS (대표):");
    println!("    translation.engine, translation.source_lang, translation.target_lang");
    println!("    translation.eztrans_dll_path, translation.eztrans_dat_path");
    println!("    translation.deepl_api_key, translation.papago_client_id,");
    println!("    translation.papago_client_secret");
    println!("    translation.llm.provider, translation.llm.model, translation.llm.api_key");
    println!(
        "    translation.llm.base_url, translation.llm.temperature, translation.llm.max_tokens"
    );
    println!("    clipboard_watch, click_through, magnetic_mode, background_visible,");
    println!("    border_visible, window_topmost, window_visible");
}

// ---------------------------------------------------------------------------
// translate
// ---------------------------------------------------------------------------

fn cmd_translate(args: &[String], json: bool) -> Result<(), String> {
    let mut text: Option<String> = None;
    let mut engine_override: Option<String> = None;
    let mut from_override: Option<String> = None;
    let mut to_override: Option<String> = None;
    let mut use_stdin = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--engine" => {
                engine_override = Some(get_value(args, &mut i, "--engine")?);
            }
            "--from" => {
                from_override = Some(get_value(args, &mut i, "--from")?);
            }
            "--to" => {
                to_override = Some(get_value(args, &mut i, "--to")?);
            }
            "--stdin" => {
                use_stdin = true;
                i += 1;
            }
            a if a.starts_with("--") => {
                return Err(format!("알 수 없는 옵션: {a}"));
            }
            _ => {
                if text.is_none() {
                    text = Some(args[i].clone());
                } else {
                    return Err("translate 는 위치 인자로 텍스트를 하나만 받습니다.".to_string());
                }
                i += 1;
            }
        }
    }

    let text = if use_stdin {
        read_stdin_to_string()?
    } else {
        text.ok_or_else(|| "번역할 텍스트가 필요합니다. (텍스트 인자 또는 --stdin)".to_string())?
    };
    let text = text.trim_end_matches(['\r', '\n']).to_string();
    if text.is_empty() {
        return Err("빈 텍스트는 번역할 수 없습니다.".to_string());
    }

    let config = Config::load_or_default();
    let engine = resolve_engine(&engine_override, &config)?;
    let (source_lang, target_lang) =
        resolve_languages(&from_override, &to_override, &config, engine)?;

    let translated = run_translation(&config, engine, source_lang, target_lang, &text)?;

    if json {
        println!(
            "{}",
            json_object(&[
                ("engine", JsonVal::Str(engine.to_str())),
                ("source", JsonVal::Str(lang_utils::to_code(source_lang))),
                ("target", JsonVal::Str(lang_utils::to_code(target_lang))),
                ("input", JsonVal::Str(&text)),
                ("output", JsonVal::Str(&translated)),
            ])
        );
    } else {
        println!("{translated}");
    }
    Ok(())
}

/// 엔진/언어를 받아 실제 번역을 수행. HTTP 엔진은 별도 tokio 런타임에서
/// `translate_async` 를 `block_on` 한다 (디스패치 큐 우회).
fn run_translation(
    config: &Config,
    engine: TranslationEngine,
    source: Language,
    target: Language,
    text: &str,
) -> Result<String, String> {
    match engine {
        TranslationEngine::EzTrans => translate_via_eztrans(config, source, target, text),
        _ => translate_via_http(config, engine, source, target, text),
    }
}

fn translate_via_eztrans(
    config: &Config,
    source: Language,
    target: Language,
    text: &str,
) -> Result<String, String> {
    let defaults = crate::config::TranslationConfig::default();
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
        return Err("eztrans 경로를 확인할 수 없습니다. config.toml 의 eztrans_dll_path/eztrans_dat_path 를 설정하세요.".to_string());
    }
    {
        let manager = get_eztrans_manager();
        let mut mgr = manager
            .lock()
            .map_err(|_| "EzTrans 매니저 잠금 실패".to_string())?;
        mgr.init(dll, dat)
            .map_err(|e| format!("EzTrans 초기화 실패: {e}"))?;
    }
    translate_with_eztrans(text, source, target).map_err(|e| format!("번역 실패: {e}"))
}

fn translate_via_http(
    config: &Config,
    engine: TranslationEngine,
    source: Language,
    target: Language,
    text: &str,
) -> Result<String, String> {
    let credentials = build_credentials(engine, config)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio 런타임 생성 실패: {e}"))?;
    let client = shared_client();
    let req = TranslationRequest {
        id: 0,
        text: Arc::from(text),
        engine,
        source_lang: source,
        target_lang: target,
        credentials,
    };
    rt.block_on(TranslationDispatch::translate_async(&req, &client))
        .map_err(|e| format!("번역 실패: {e}"))
}

fn build_credentials(
    engine: TranslationEngine,
    config: &Config,
) -> Result<EngineCredentials, String> {
    Ok(match engine {
        TranslationEngine::EzTrans | TranslationEngine::Google => EngineCredentials::None,
        TranslationEngine::DeepL => {
            let keys = config.translation.deepl_effective_keys();
            if keys.is_empty() {
                return Err("DeepL API 키가 설정되지 않았습니다. config set translation.deepl_api_key <KEY>".to_string());
            }
            EngineCredentials::DeepL {
                keys,
                strategy: config.translation.deepl_strategy(),
            }
        }
        TranslationEngine::Papago => {
            if config.translation.papago_client_id.is_empty()
                || config.translation.papago_client_secret.is_empty()
            {
                return Err("Papago client_id/client_secret 가 설정되지 않았습니다.".to_string());
            }
            EngineCredentials::Papago {
                client_id: config.translation.papago_client_id.clone(),
                client_secret: config.translation.papago_client_secret.clone(),
            }
        }
        TranslationEngine::Llm => {
            if config.translation.llm.api_key.is_empty() {
                return Err("LLM api_key 가 설정되지 않았습니다.".to_string());
            }
            EngineCredentials::Llm(config.translation.llm.to_call_params())
        }
    })
}

fn resolve_engine(
    override_value: &Option<String>,
    config: &Config,
) -> Result<TranslationEngine, String> {
    match override_value {
        Some(s) => Ok(TranslationEngine::from_str(s)),
        None => Ok(config.translation.get_engine()),
    }
}

fn resolve_languages(
    from: &Option<String>,
    to: &Option<String>,
    config: &Config,
    engine: TranslationEngine,
) -> Result<(Language, Language), String> {
    let source = match from {
        Some(s) => {
            lang_utils::from_code(s).ok_or_else(|| format!("알 수 없는 소스 언어 코드: {s}"))?
        }
        None => config.translation.get_source_language(),
    };
    let target = match to {
        Some(s) => {
            lang_utils::from_code(s).ok_or_else(|| format!("알 수 없는 타겟 언어 코드: {s}"))?
        }
        None => config.translation.get_target_language(),
    };
    // EzTrans 는 JP->KR 만 — 다른 조합이면 명확히 거부.
    if engine == TranslationEngine::EzTrans && (source != Language::Jpn || target != Language::Kor)
    {
        return Err("EzTrans 는 일본어(ja) → 한국어(ko) 만 지원합니다.".to_string());
    }
    Ok((source, target))
}

// ---------------------------------------------------------------------------
// file-trans
// ---------------------------------------------------------------------------

fn cmd_file_trans(args: &[String], json: bool) -> Result<(), String> {
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

// ---------------------------------------------------------------------------
// list-engines / list-langs
// ---------------------------------------------------------------------------

fn cmd_list_engines(json: bool) -> Result<(), String> {
    const ENGINES: &[TranslationEngine] = &[
        TranslationEngine::EzTrans,
        TranslationEngine::Google,
        TranslationEngine::DeepL,
        TranslationEngine::Papago,
        TranslationEngine::Llm,
    ];
    if json {
        let entries: Vec<String> = ENGINES
            .iter()
            .map(|e| format!("\"{}\"", e.to_str()))
            .collect();
        println!("[{}]", entries.join(","));
    } else {
        for e in ENGINES {
            println!("{}", e.to_str());
        }
    }
    Ok(())
}

fn cmd_list_langs(args: &[String], json: bool) -> Result<(), String> {
    let mut engine_override: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--engine" => engine_override = Some(get_value(args, &mut i, "--engine")?),
            a => return Err(format!("알 수 없는 옵션: {a}")),
        }
    }
    let engine = engine_override
        .as_deref()
        .map(TranslationEngine::from_str)
        .unwrap_or(TranslationEngine::Google);
    let source = engine.supported_source_languages();
    let target = engine.supported_target_languages();

    if json {
        let to_arr = |langs: &[Language]| -> String {
            let parts: Vec<String> = langs
                .iter()
                .map(|l| format!("\"{}\"", lang_utils::to_code(*l)))
                .collect();
            format!("[{}]", parts.join(","))
        };
        println!(
            "{{\"engine\":\"{}\",\"source\":{},\"target\":{}}}",
            engine.to_str(),
            to_arr(source),
            to_arr(target),
        );
    } else {
        println!("engine: {}", engine.to_str());
        println!("source:");
        for l in source {
            println!(
                "  {} ({})",
                lang_utils::to_code(*l),
                lang_utils::to_korean_name(*l)
            );
        }
        println!("target:");
        for l in target {
            println!(
                "  {} ({})",
                lang_utils::to_code(*l),
                lang_utils::to_korean_name(*l)
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

fn cmd_config(args: &[String], json: bool) -> Result<(), String> {
    let sub = args
        .first()
        .ok_or_else(|| "config <show|get|set> ... 형식으로 호출하세요.".to_string())?;
    match sub.as_str() {
        "show" => {
            let config = Config::load_or_default();
            let toml =
                toml::to_string_pretty(&config).map_err(|e| format!("TOML 직렬화 실패: {e}"))?;
            if json {
                let json_text =
                    serde_json::to_string(&config).map_err(|e| format!("JSON 직렬화 실패: {e}"))?;
                println!("{json_text}");
            } else {
                print!("{toml}");
            }
            Ok(())
        }
        "get" => {
            let key = args
                .get(1)
                .ok_or_else(|| "config get <KEY> — 점 경로 키가 필요합니다.".to_string())?;
            let config = Config::load_or_default();
            let value = config_get(&config, key)?;
            if json {
                println!(
                    "{}",
                    json_object(&[("key", JsonVal::Str(key)), ("value", JsonVal::Str(&value))])
                );
            } else {
                println!("{value}");
            }
            Ok(())
        }
        "set" => {
            let key = args
                .get(1)
                .ok_or_else(|| "config set <KEY> <VALUE> — 키가 필요합니다.".to_string())?;
            let value = args
                .get(2)
                .ok_or_else(|| "config set <KEY> <VALUE> — 값이 필요합니다.".to_string())?;
            let mut config = Config::load_or_default();
            config_set(&mut config, key, value)?;
            config
                .save()
                .map_err(|e| format!("config 저장 실패: {e}"))?;
            if json {
                println!(
                    "{}",
                    json_object(&[
                        ("key", JsonVal::Str(key)),
                        ("value", JsonVal::Str(value)),
                        ("saved", JsonVal::Bool(true)),
                    ])
                );
            } else {
                println!("저장됨: {key} = {value}");
            }
            Ok(())
        }
        other => Err(format!("알 수 없는 config 서브커맨드: {other}")),
    }
}

fn cmd_config_path(json: bool) -> Result<(), String> {
    let path = Config::default_config_path();
    let s = path.to_string_lossy();
    if json {
        println!("{}", json_object(&[("path", JsonVal::Str(&s))]));
    } else {
        println!("{s}");
    }
    Ok(())
}

/// 점 경로로 config 값을 문자열로 조회한다. 점 경로 분기를 위해
/// `serde_json::Value` 트리로 한 번 변환한 뒤 탐색해 값을 꺼내는 단순 구현.
/// 깊은 중첩(예: `translation.llm.temperature`)도 자연스럽게 처리된다.
fn config_get(config: &Config, key: &str) -> Result<String, String> {
    let v = serde_json::to_value(config).map_err(|e| format!("config 직렬화 실패: {e}"))?;
    let mut cur = &v;
    for part in key.split('.') {
        match cur {
            serde_json::Value::Object(map) => match map.get(part) {
                Some(next) => cur = next,
                None => return Err(format!("키를 찾을 수 없습니다: {key} (segment: {part})")),
            },
            _ => return Err(format!("키 경로가 객체가 아닙니다: {key}")),
        }
    }
    Ok(match cur {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    })
}

/// 점 경로로 config 값을 설정. 직렬화→수정→역직렬화 패턴.
/// `translation.llm.glossary` 같은 배열 필드는 set 대상에서 제외(현재 CLI 범위 밖).
fn config_set(config: &mut Config, key: &str, value: &str) -> Result<(), String> {
    let mut v = serde_json::to_value(&*config).map_err(|e| format!("config 직렬화 실패: {e}"))?;
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        return Err("빈 키".to_string());
    }

    // 마지막 부분 직전까지 mutable 포인터 이동
    let (last, prefix) = parts.split_last().expect("not empty");
    let mut cur = &mut v;
    for part in prefix {
        match cur {
            serde_json::Value::Object(map) => {
                cur = map
                    .get_mut(*part)
                    .ok_or_else(|| format!("키를 찾을 수 없습니다: {key} (segment: {part})"))?;
            }
            _ => return Err(format!("키 경로가 객체가 아닙니다: {key}")),
        }
    }
    let target_obj = match cur {
        serde_json::Value::Object(map) => map,
        _ => return Err(format!("키 경로가 객체가 아닙니다: {key}")),
    };
    let slot = target_obj
        .get_mut(*last)
        .ok_or_else(|| format!("키를 찾을 수 없습니다: {key} (segment: {last})"))?;

    // 기존 값의 타입을 보존해 파싱한다. (예: bool 필드에는 "true"/"false"만 허용)
    let new_val = coerce_value(slot, value).map_err(|e| format!("값 변환 실패 ({key}): {e}"))?;
    *slot = new_val;

    let new_config: Config =
        serde_json::from_value(v).map_err(|e| format!("config 역직렬화 실패: {e}"))?;
    *config = new_config;
    Ok(())
}

fn coerce_value(existing: &serde_json::Value, raw: &str) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    match existing {
        Value::Bool(_) => match raw.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(Value::Bool(true)),
            "false" | "0" | "no" | "off" => Ok(Value::Bool(false)),
            _ => Err(format!("bool 값으로 변환할 수 없습니다: {raw}")),
        },
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                let parsed: i64 = raw
                    .parse()
                    .map_err(|_| format!("정수로 변환할 수 없습니다: {raw}"))?;
                Ok(Value::Number(parsed.into()))
            } else {
                let parsed: f64 = raw
                    .parse()
                    .map_err(|_| format!("실수로 변환할 수 없습니다: {raw}"))?;
                let num = serde_json::Number::from_f64(parsed)
                    .ok_or_else(|| "NaN/Inf 는 허용되지 않습니다".to_string())?;
                Ok(Value::Number(num))
            }
        }
        Value::String(_) => Ok(Value::String(raw.to_string())),
        Value::Null => Ok(Value::String(raw.to_string())),
        Value::Array(_) | Value::Object(_) => Err(
            "배열/객체 필드는 CLI 에서 직접 수정할 수 없습니다. config.toml 을 직접 편집하세요."
                .to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn get_value(args: &[String], i: &mut usize, name: &str) -> Result<String, String> {
    let next = args
        .get(*i + 1)
        .cloned()
        .ok_or_else(|| format!("{name} 뒤에 값이 필요합니다."))?;
    *i += 2;
    Ok(next)
}

fn read_stdin_to_string() -> Result<String, String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("stdin 읽기 실패: {e}"))?;
    Ok(buf)
}

/// 최소한의 JSON 출력 헬퍼. 외부 의존성 없이 짧은 객체만 출력하므로
/// 문자열 이스케이프만 직접 구현한다.
enum JsonVal<'a> {
    Str(&'a str),
    Bool(bool),
    Num(i64),
}

fn json_object(entries: &[(&str, JsonVal<'_>)]) -> String {
    let mut s = String::from("{");
    for (i, (k, v)) in entries.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('"');
        s.push_str(k);
        s.push_str("\":");
        match v {
            JsonVal::Str(value) => {
                s.push('"');
                json_escape_into(&mut s, value);
                s.push('"');
            }
            JsonVal::Bool(b) => s.push_str(if *b { "true" } else { "false" }),
            JsonVal::Num(n) => s.push_str(&n.to_string()),
        }
    }
    s.push('}');
    s
}

fn json_escape_into(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
}
