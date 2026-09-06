use crate::translation::{TranslationEngine, lang_utils};

#[derive(clap::Args)]
pub(super) struct LanguagesArgs {
    /// 조회할 번역 엔진
    #[arg(long, value_enum)]
    engine: Option<super::Engine>,
}

pub(super) fn engines() -> Result<(), String> {
    const ENGINES: &[TranslationEngine] = &[
        TranslationEngine::EzTrans,
        TranslationEngine::Google,
        TranslationEngine::DeepL,
        TranslationEngine::Papago,
        TranslationEngine::Llm,
        TranslationEngine::MysTranslater,
        TranslationEngine::Custom,
    ];
    for e in ENGINES {
        println!("{}", e.to_str());
    }
    Ok(())
}

pub(super) fn languages(args: LanguagesArgs) -> Result<(), String> {
    let engine = args
        .engine
        .map_or(TranslationEngine::Google, TranslationEngine::from);
    let source = engine.supported_source_languages();
    let target = engine.supported_target_languages();

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
    Ok(())
}
