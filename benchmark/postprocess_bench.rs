//! 파일 번역 postprocess 사전 스캔 벤치마크 (2-h 측정).
//!
//! `EzTransPostprocessMatcher`는 작업 준비 시 automaton을 1회 빌드하고,
//! 번역 결과마다 `apply()`를 호출한다. 사전 크기별·매칭 밀도별로 apply 비용을
//! 실측해 자동화 전환 전후를 비교한다.
//!
//! 비교 대상:
//! - `matcher`: automaton 1회 빌드 후 재사용 (현재 앱 경로)
//! - `rebuild`: 매 호출마다 automaton 빌드 (전환 전 브루트포스와 대조용)

use anemone_rs::BenchmarkEzTransPostprocessEntry as EzTransPostprocessEntry;
use anemone_rs::translation::{
    BenchmarkEzTransPostprocessMatcher as EzTransPostprocessMatcher,
    benchmark_apply_eztrans_dictionary,
};
use std::time::{Duration, Instant};

fn entry(source: &str, target: &str) -> EzTransPostprocessEntry {
    EzTransPostprocessEntry {
        source: source.into(),
        target: target.into(),
    }
}

/// 사전 항목 생성 — 실제 용법과 유사하게 한 줄 문장 패턴으로 만든다.
fn build_dictionary(size: usize) -> Vec<EzTransPostprocessEntry> {
    (0..size)
        .map(|i| {
            entry(
                &format!("これはテスト文{i:04}です"),
                &format!("이것은 테스트문{i:04}입니다"),
            )
        })
        .collect()
}

/// 번역 결과처럼 생긴 줄. 매칭 밀도로 시나리오를 나눈다.
fn build_line(matching_entries: usize, line_len: usize) -> String {
    // 사전 항목을 반복해 배치해 넣고 나머지는 평문으로 채운다.
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while parts.iter().map(std::string::String::len).sum::<usize>() < line_len {
        if i % 3 == 0 && matching_entries > 0 {
            parts.push(format!("これはテスト文{:04}です", i % matching_entries));
            i += 1;
        } else {
            parts.push("日本語の普通の文章が続きます。".to_string());
            i += 1;
        }
    }
    let mut text = parts.concat();
    while text.len() > line_len {
        text.pop();
    }
    text
}

fn report(label: &str, elapsed: Duration, units: usize) {
    let per_us = elapsed.as_secs_f64() * 1_000_000.0 / units as f64;
    eprintln!(
        "[bench {label}] n={units} total={:.1}ms per={:.1}us",
        elapsed.as_secs_f64() * 1000.0,
        per_us
    );
}

/// 사전 크기 × 매칭 밀도 조합을 200줄 × 반복으로 측정한다.
#[test]
#[ignore = "performance benchmark that measures postprocess dictionary scan cost"]
fn measures_postprocess_dictionary_scan() {
    assert!(
        !std::hint::black_box(cfg!(debug_assertions)),
        "performance measurements must run with cargo test --release"
    );

    const LINES: usize = 200;
    const REPEAT: usize = 20;

    for dict_size in [100usize, 1_000, 5_000] {
        let dictionary = build_dictionary(dict_size);
        let matcher = EzTransPostprocessMatcher::new(&dictionary)
            .expect("non-empty dictionary builds an automaton");
        // 각 줄을 한 번만 만들어 재사용한다 (postprocess 비용만 분리).
        let lines = [
            build_line(dict_size, 200),      // 전부 매칭 (사전이 줄마다 걸림)
            build_line(dict_size / 10, 200), // 일부 매칭
            build_line(0, 200),              // 전부 미매칭
        ];
        let labels = ["all-match", "partial", "no-match"];
        for (line, label) in lines.iter().zip(labels) {
            // 현재 앱 경로: matcher 1회 빌드, apply만 반복.
            let started = Instant::now();
            let mut sink = 0usize;
            for _ in 0..REPEAT {
                for _ in 0..LINES {
                    let out = matcher.apply(line.clone());
                    sink += out.len();
                }
            }
            report(
                &format!("postprocess-{label}-dict{dict_size}"),
                started.elapsed(),
                LINES * REPEAT,
            );
            std::hint::black_box(sink);

            // 대조: 매 호출마다 재빌드 (apply_eztrans_dictionary 경로).
            let started = Instant::now();
            let mut sink = 0usize;
            for _ in 0..REPEAT {
                for _ in 0..LINES {
                    let out = benchmark_apply_eztrans_dictionary(line.clone(), &dictionary);
                    sink += out.len();
                }
            }
            report(
                &format!("postprocess-rebuild-{label}-dict{dict_size}"),
                started.elapsed(),
                LINES * REPEAT,
            );
            std::hint::black_box(sink);
        }
    }
}
