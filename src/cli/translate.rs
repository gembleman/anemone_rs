use std::io::Read;
use std::sync::Arc;

use crate::config::Config;
use crate::translation::http_common::shared_client;
use crate::translation::worker::{TranslationDispatch, TranslationRequest};
use crate::translation::{
    EngineCredentials, Language, TranslationEngine, get_eztrans_manager, lang_utils,
    translate_with_eztrans,
};

use super::helpers::{JsonVal, json_object};

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

pub(super) fn run(args: Args, json: bool) -> Result<(), String> {
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
    let engine = resolve_engine(args.engine, &config);
    let (source_lang, target_lang) =
        resolve_languages(&args.source, &args.target, &config, engine)?;

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

pub(super) fn build_credentials(
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

pub(super) fn resolve_engine(
    override_value: Option<super::Engine>,
    config: &Config,
) -> TranslationEngine {
    match override_value {
        Some(engine) => engine.into(),
        None => config.translation.get_engine(),
    }
}

pub(super) fn resolve_languages(
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

fn read_stdin_to_string() -> Result<String, String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("stdin 읽기 실패: {e}"))?;
    Ok(buf)
}
