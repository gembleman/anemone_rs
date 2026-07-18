use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use super::*;

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
        });
    });

    let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(!output.contains(clipboard_secret), "{output}");
    assert!(!output.contains(api_secret), "{output}");
    assert!(output.contains("clipboard_chars=23"), "{output}");
    assert!(output.contains("status_code=Some(401)"), "{output}");
}
