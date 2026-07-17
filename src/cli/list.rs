use crate::translation::{Language, TranslationEngine, lang_utils};

use super::helpers::get_value;

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

pub(super) fn languages(args: &[String], json: bool) -> Result<(), String> {
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
