use super::{
    BoundedTranslationCache, read_input_line, translate_eztrans_window, validate_and_count_reader,
};
use crate::translation::BenchmarkEzTransBatchTranslator as EzTransBatchTranslator;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct MockBatchTranslator {
    process_count: usize,
    translated_lines: AtomicUsize,
}

impl EzTransBatchTranslator for MockBatchTranslator {
    fn process_count(&self) -> usize {
        self.process_count
    }

    fn translate_batches(
        &self,
        batches: Vec<Vec<Arc<str>>>,
        _cancelled: &Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Vec<Vec<String>>, String> {
        self.translated_lines.fetch_add(
            batches.iter().map(Vec::len).sum::<usize>(),
            Ordering::Relaxed,
        );
        Ok(batches
            .into_iter()
            .map(|batch| {
                batch
                    .into_iter()
                    .map(|text| format!("번역:{text}"))
                    .collect()
            })
            .collect())
    }
}

fn eztrans_job() -> crate::file_trans::FileTranslationRequest {
    use crate::translation::{Language, PreparedJob};
    crate::file_trans::FileTranslationRequest {
        input_files: Vec::new(),
        output_files: Vec::new(),
        write_type: super::WriteType::TranslationOnly,
        no_trans_linefeed: true,
        cancel_token: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        translation: PreparedJob::eztrans(
            "test.dll".into(),
            "test-dat".into(),
            1,
            Language::Jpn,
            Language::Kor,
        )
        .unwrap(),
    }
}

#[test]
#[ignore = "benchmark that processes all 200,000 sample lines"]
fn large_fixture_streams_in_bounded_windows_and_translates_each_unique_line_once() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benchmark")
        .join("large_japanese_translation_sample.txt");
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let reader = BufReader::new(File::open(&path).unwrap());
    assert_eq!(
        validate_and_count_reader(reader, &path, &cancel).unwrap(),
        200_000
    );

    let translator = MockBatchTranslator {
        process_count: 8,
        translated_lines: AtomicUsize::new(0),
    };
    let job = eztrans_job();
    let mut cache = BoundedTranslationCache::new(100_000);
    let mut reader = BufReader::new(File::open(&path).unwrap());
    let mut first_line = true;
    let mut processed = 0usize;
    loop {
        let mut window = Vec::with_capacity(50_000);
        while window.len() < 50_000 {
            let Some(line) = read_input_line(&mut reader, &path, first_line).unwrap() else {
                break;
            };
            first_line = false;
            window.push(line);
        }
        if window.is_empty() {
            break;
        }
        let translated = translate_eztrans_window(&window, &job, &translator, &mut cache).unwrap();
        assert_eq!(translated.len(), window.len());
        assert_eq!(translated[0], format!("번역:{}", window[0].text));
        processed += translated.len();
    }

    assert_eq!(processed, 200_000);
    assert_eq!(translator.translated_lines.load(Ordering::Relaxed), 10_000);
}

#[test]
#[ignore = "benchmark fixture integrity check"]
fn unique_fixture_contains_ten_thousand_distinct_sentences() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benchmark")
        .join("unique_japanese_translation_sample.txt");
    let contents = std::fs::read_to_string(path).unwrap();
    let lines = contents.lines().collect::<Vec<_>>();
    let unique = lines
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();

    assert_eq!(lines.len(), 10_000);
    assert_eq!(unique.len(), 10_000);
}
