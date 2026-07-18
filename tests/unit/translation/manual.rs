use super::*;

#[test]
fn prepare_input_normalizes_crlf_and_standalone_lf() {
    let options = ManualTranslationOptions {
        remove_linefeeds: true,
        ..ManualTranslationOptions::default()
    };

    assert_eq!(
        options.prepare_input("첫째\r\n둘째\n셋째"),
        "첫째 둘째 셋째"
    );
}

#[test]
fn prepare_input_preserves_linefeeds_when_disabled() {
    let options = ManualTranslationOptions::default();
    assert_eq!(
        options.prepare_input("첫째\r\n둘째\n셋째"),
        "첫째\r\n둘째\n셋째"
    );
}

#[test]
fn formats_normal_and_bracketed_output() {
    let normal = ManualTranslationOptions::default();
    assert_eq!(normal.format_output("번역".into()), "번역");

    let brackets = ManualTranslationOptions {
        output_format: ManualOutputFormat::Brackets,
        ..ManualTranslationOptions::default()
    };
    assert_eq!(brackets.format_output("번역".into()), "「번역」");
}

#[test]
fn name_split_accepts_ascii_and_fullwidth_colons() {
    let options = ManualTranslationOptions {
        output_format: ManualOutputFormat::NameSplit,
        ..ManualTranslationOptions::default()
    };

    assert_eq!(options.format_output("Alice: Hello".into()), "Alice\nHello");
    assert_eq!(
        options.format_output("앨리스： 안녕".into()),
        "앨리스\n안녕"
    );
}

#[test]
fn name_split_preserves_output_without_a_colon() {
    let options = ManualTranslationOptions {
        output_format: ManualOutputFormat::NameSplit,
        ..ManualTranslationOptions::default()
    };

    assert_eq!(
        options.format_output("구분자 없는 번역".into()),
        "구분자 없는 번역"
    );
}
