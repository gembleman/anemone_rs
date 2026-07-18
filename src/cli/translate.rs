use std::io::Read;
use std::sync::Arc;

use crate::config::Config;
use crate::translation::http_common::shared_client;
use crate::translation::worker::{TranslationDispatch, TranslationRequest};
use crate::translation::{Language, TranslationEngine, TranslationJobSpec, lang_utils};

#[derive(clap::Args)]
pub(super) struct Args {
    /// 번역할 텍스트
    #[arg(required_unless_present = "stdin", conflicts_with = "stdin")]
    text: Option<String>,
    /// 사용할 번역 엔진
    #[arg(long, value_enum)]
    engine: Option<super::Engine>,
    /// 소스 언어 코드 (예: ja)
    #[arg(long = "from")]
    source: Option<String>,
    /// 타겟 언어 코드 (예: ko)
    #[arg(long = "to")]
    target: Option<String>,
    /// stdin에서 번역할 텍스트 읽기
    #[arg(long)]
    stdin: bool,
}

pub(super) fn run(args: Args) -> Result<(), String> {
    let text = if args.stdin {
        read_stdin_to_string()?
    } else {
        args.text
            .expect("clap이 텍스트 또는 --stdin 중 하나를 보장해야 함")
    };
    let text = text.trim_end_matches(['\r', '\n']).to_string();
    if text.is_empty() {
        return Err("빈 텍스트는 번역할 수 없습니다.".to_string());
    }

    let config = Config::load_or_default();
    let engine = resolve_engine(args.engine, &config)?;
    let (source_lang, target_lang) = resolve_languages(&args.source, &args.target, &config)?;

    let spec = TranslationJobSpec::with_engine_languages(
        &config.translation,
        engine,
        source_lang,
        target_lang,
    )
    .map_err(|error| error.to_string())?;
    let translated = run_translation(spec, &text)?;

    println!("{translated}");
    Ok(())
}

/// 엔진/언어를 받아 실제 번역을 수행. HTTP 엔진은 별도 tokio 런타임에서
/// `translate_async` 를 `block_on` 한다 (디스패치 큐 우회).
fn run_translation(spec: TranslationJobSpec, text: &str) -> Result<String, String> {
    spec.prepare().map_err(|error| error.to_string())?;
    let (engine, source_lang, target_lang, credentials) = spec.into_parts();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio 런타임 생성 실패: {e}"))?;
    let client = shared_client();
    let req = TranslationRequest {
        id: 0,
        text: Arc::from(text),
        engine,
        source_lang,
        target_lang,
        credentials,
    };
    rt.block_on(TranslationDispatch::translate_async(&req, &client))
        .map_err(|e| format!("번역 실패: {e}"))
}

pub(super) fn resolve_engine(
    override_value: Option<super::Engine>,
    config: &Config,
) -> Result<TranslationEngine, String> {
    match override_value {
        Some(engine) => Ok(engine.into()),
        None => config
            .translation
            .get_engine()
            .map_err(|error| error.to_string()),
    }
}

pub(super) fn resolve_languages(
    from: &Option<String>,
    to: &Option<String>,
    config: &Config,
) -> Result<(Language, Language), String> {
    let source = match from {
        Some(s) => {
            lang_utils::from_code(s).ok_or_else(|| format!("알 수 없는 소스 언어 코드: {s}"))?
        }
        None => config
            .translation
            .get_source_language()
            .map_err(|error| error.to_string())?,
    };
    let target = match to {
        Some(s) => {
            lang_utils::from_code(s).ok_or_else(|| format!("알 수 없는 타겟 언어 코드: {s}"))?
        }
        None => config
            .translation
            .get_target_language()
            .map_err(|error| error.to_string())?,
    };
    Ok((source, target))
}

fn read_stdin_to_string() -> Result<String, String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("stdin 읽기 실패: {e}"))?;
    Ok(buf)
}
