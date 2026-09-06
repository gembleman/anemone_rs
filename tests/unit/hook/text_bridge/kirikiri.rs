use super::*;

#[test]
fn kirikiri_postprocess_collapses_duplicate_speaker_prefix() {
    let mut text = "【お天気お姉さん】【お天気お姉さん】「本日の降水確率は０％」".to_string();
    postprocess_hook_text("KiriKiri", &mut text);
    assert_eq!(text, "【お天気お姉さん】\n「本日の降水確率は０％」");
}

#[test]
fn kirikiri_postprocess_breaks_line_after_speaker_prefix() {
    let mut text = "【お天気お姉さん】「本日の降水確率は０％」".to_string();
    postprocess_hook_text("KiriKiri", &mut text);
    assert_eq!(text, "【お天気お姉さん】\n「本日の降水確率は０％」");
}

#[test]
fn speaker_line_break_normalizes_existing_separators() {
    // 이미 개행/공백으로 나뉜 문장은 개행 하나로 정리한다.
    let mut spaced = "【姉】 本文".to_string();
    break_after_speaker_prefix(&mut spaced);
    assert_eq!(spaced, "【姉】\n本文");

    let mut broken = "【姉】\n本文".to_string();
    break_after_speaker_prefix(&mut broken);
    assert_eq!(broken, "【姉】\n本文");
}

#[test]
fn speaker_line_break_skips_prefix_only_and_unnamed_text() {
    // 화자 표기뿐인 문장에 개행을 붙이면 빈 줄만 남는다.
    let mut prefix_only = "【姉】".to_string();
    break_after_speaker_prefix(&mut prefix_only);
    assert_eq!(prefix_only, "【姉】");

    let mut empty_name = "【】本文".to_string();
    break_after_speaker_prefix(&mut empty_name);
    assert_eq!(empty_name, "【】本文");

    let mut no_prefix = "本文【姉】".to_string();
    break_after_speaker_prefix(&mut no_prefix);
    assert_eq!(no_prefix, "本文【姉】");
}

#[test]
fn non_kirikiri_postprocess_leaves_speaker_prefix_untouched() {
    let original = "【姉】【姉】本文";
    let mut text = original.to_string();
    postprocess_hook_text("BGI", &mut text);
    assert_eq!(text, original);
}

#[test]
fn merger_preserves_distinct_speaker_prefixes_and_mid_sentence_repetitions() {
    let mut distinct = "【姉】【妹】本文".to_string();
    collapse_duplicate_speaker_prefix(&mut distinct);
    assert_eq!(distinct, "【姉】【妹】本文");

    let mut mid_sentence = "本文【姉】【姉】".to_string();
    collapse_duplicate_speaker_prefix(&mut mid_sentence);
    assert_eq!(mid_sentence, "本文【姉】【姉】");
}

#[test]
fn merger_collapses_all_consecutive_duplicate_speaker_prefixes() {
    let mut text = "【姉】【姉】【姉】本文".to_string();
    collapse_duplicate_speaker_prefix(&mut text);
    assert_eq!(text, "【姉】本文");
}

#[test]
fn recognizes_only_kirikiri_engine_names() {
    for name in ["KiriKiri", "KiriKiri1", "KiriKiriZ", "krkrz64", "Krkr2wcs"] {
        assert!(is_kirikiri_engine(name), "{name}");
    }
    for name in ["V8", "RenPy", "BGI", "UserUI"] {
        assert!(!is_kirikiri_engine(name), "{name}");
    }
}
