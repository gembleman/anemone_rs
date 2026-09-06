use std::io::Read;
use std::sync::Arc;

use crate::config::Config;
use crate::translation::{
    Language, PreparedJob, TranslationEngine, TranslationService, lang_utils,
};

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
    run_with_config(args, Config::load_or_default())
}

/// `run`에서 설정 로드를 분리해, 테스트가 실제 사용자 설정 파일을 건드리지
/// 않고 인자 파싱 이후의 번역 실행 경로(엔진/언어 결정, 실제 번역 호출)를
/// 검증할 수 있게 한다.
pub(super) fn run_with_config(args: Args, mut config: Config) -> Result<(), String> {
    config
        .translation
        .activate_route(crate::config::TranslationRoute::Manual);
    let text = resolve_text(args.text, args.stdin, read_stdin_to_string)?;

    let engine = resolve_engine(args.engine, &config)?;
    let (source_lang, target_lang) = resolve_languages(&args.source, &args.target, &config)?;

    let job =
        PreparedJob::with_engine_languages(&config.translation, engine, source_lang, target_lang)
            .map_err(|error| error.to_string())?;
    let translated = run_translation(job, &text)?;

    println!("{translated}");
    Ok(())
}

/// 엔진/언어를 받아 실제 번역을 수행. HTTP 엔진은 별도 tokio 런타임에서
/// `translate_async` 를 `block_on` 한다 (디스패치 큐 우회).
fn run_translation(job: PreparedJob, text: &str) -> Result<String, String> {
    job.prepare().map_err(|error| error.to_string())?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio 런타임 생성 실패: {e}"))?;
    let service = TranslationService::new();
    rt.block_on(service.translate(job, Arc::from(text)))
        .map_err(|e| format!("번역 실패: {e}"))
}

/// `--stdin`/텍스트 인자 분기와 개행 trim, 빈 텍스트 거부를 모아 처리한다.
/// 실제 stdin 읽기는 `read_stdin`으로 주입해 테스트에서 대체할 수 있게 한다.
pub(super) fn resolve_text(
    text: Option<String>,
    use_stdin: bool,
    read_stdin: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    let raw = if use_stdin {
        read_stdin()?
    } else {
        text.ok_or_else(|| {
            "번역할 텍스트가 없습니다. 텍스트를 인자로 주거나 --stdin을 사용하세요.".to_string()
        })?
    };
    let trimmed = raw.trim_end_matches(['\r', '\n']).to_string();
    if trimmed.is_empty() {
        return Err("빈 텍스트는 번역할 수 없습니다.".to_string());
    }
    Ok(trimmed)
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

#[cfg(test)]
#[path = "../../tests/unit/cli/translate.rs"]
mod tests;
