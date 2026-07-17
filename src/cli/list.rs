use crate::translation::{Language, TranslationEngine, lang_utils};

#[derive(clap::Args)]
pub(super) struct LanguagesArgs {
    /// 조회할 번역 엔진
    #[arg(long, value_enum)]
    engine: Option<super::Engine>,
}

pub(super) fn engines(json: bool) -> Result<(), String> {
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

pub(super) fn languages(args: LanguagesArgs, json: bool) -> Result<(), String> {
    let engine = args
        .engine
        .map(TranslationEngine::from)
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
