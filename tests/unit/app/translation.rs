use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use super::*;

#[test]
fn debounce_covers_paid_engines_and_llm() {
    assert_eq!(debounce_delay_ms(TranslationEngine::Llm, 300), 300);
    assert_eq!(debounce_delay_ms(TranslationEngine::Llm, 0), 0);
    assert_eq!(
        debounce_delay_ms(TranslationEngine::Google, 300),
        PAID_ENGINE_DEBOUNCE_MS
    );
    assert_eq!(
        debounce_delay_ms(TranslationEngine::DeepL, 300),
        PAID_ENGINE_DEBOUNCE_MS
    );
    assert_eq!(
        debounce_delay_ms(TranslationEngine::Papago, 300),
        PAID_ENGINE_DEBOUNCE_MS
    );
    assert_eq!(
        debounce_delay_ms(TranslationEngine::Custom, 300),
        PAID_ENGINE_DEBOUNCE_MS
    );
    assert_eq!(
        debounce_delay_ms(TranslationEngine::MysTranslater, 300),
        PAID_ENGINE_DEBOUNCE_MS
    );
    assert_eq!(debounce_delay_ms(TranslationEngine::EzTrans, 300), 0);
}

#[test]
fn source_language_guard_only_skips_text_without_the_source_script() {
    use crate::translation::Language;

    // 소스가 일본어면 일본어 문자가 없는 원문만 번역을 건너뛴다.
    assert!(bypasses_translation(true, Language::Jpn, "안녕하세요"));
    assert!(bypasses_translation(true, Language::Jpn, "Save / Load"));
    assert!(!bypasses_translation(true, Language::Jpn, "こんにちは"));
    assert!(!bypasses_translation(true, Language::Jpn, "選択肢"));

    // 옵션이 꺼져 있으면 어떤 원문도 거르지 않는다.
    assert!(!bypasses_translation(false, Language::Jpn, "안녕하세요"));

    // 소스가 영어면 반대 방향으로 걸린다.
    assert!(bypasses_translation(true, Language::Eng, "こんにちは"));
    assert!(!bypasses_translation(true, Language::Eng, "Save / Load"));
}

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn logs_never_include_clipboard_or_api_error_secrets() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let sink = bytes.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || Capture(sink.clone()))
        .finish();
    let clipboard_secret = "clipboard-secret-79b861";
    let api_secret = "sk-api-secret-2f2ba9";

    tracing::subscriber::with_default(subscriber, || {
        log_clipboard_metadata(clipboard_secret);
        log_translation_failure(&crate::translation::TranslationError::Api {
            code: 401,
            message: format!("Authorization failed for {api_secret}"),
            retry_after: None,
        });
    });

    let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(!output.contains(clipboard_secret), "{output}");
    assert!(!output.contains(api_secret), "{output}");
    assert!(output.contains("clipboard_chars=23"), "{output}");
    assert!(output.contains("status_code=Some(401)"), "{output}");
}
