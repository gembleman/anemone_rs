use clap::{CommandFactory, Parser};
use std::ffi::OsString;

use super::{Cli, CliOutcome, run_with_argv};

#[cfg(mys_private)]
#[path = "mod_mys.rs"]
mod mys;

fn argv(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

// ---- run_with_argv: Config/네트워크에 닿지 않는 분기만 검증한다 ----
// (성공적으로 파싱되는 하위 명령은 Config::load_or_default()나 실제 번역
// 백엔드를 건드리므로 여기서는 다루지 않는다 — 파싱 실패/GUI 진입 분기만
// 결정적으로 재현 가능하다.)

#[test]
fn run_with_argv_enters_gui_mode_when_no_arguments_are_given() {
    assert!(matches!(
        run_with_argv(argv(&["anemone_rs"])),
        CliOutcome::Gui
    ));
    assert!(matches!(run_with_argv(Vec::new()), CliOutcome::Gui));
}

#[test]
fn run_with_argv_reports_the_clap_usage_error_exit_code_for_an_unknown_subcommand() {
    let outcome = run_with_argv(argv(&["anemone_rs", "이런-명령은-없습니다"]));
    assert_eq!(outcome, CliOutcome::Done(2));
}

#[test]
fn run_with_argv_reports_success_exit_code_for_help() {
    let outcome = run_with_argv(argv(&["anemone_rs", "--help"]));
    assert_eq!(outcome, CliOutcome::Done(0));
}

#[test]
fn run_with_argv_rejects_removed_config_subcommand_before_dispatch() {
    let outcome = run_with_argv(argv(&["anemone_rs", "config", "show"]));
    assert_eq!(outcome, CliOutcome::Done(2));
}

#[test]
fn run_with_argv_dispatches_list_engines_without_touching_config_or_the_network() {
    // list-engines/list-langs는 하드코딩된 목록만 출력하고 Config나 네트워크를
    // 건드리지 않으므로, run_with_argv의 dispatch 분기까지 안전하게 실행해 볼 수
    // 있는 유일한 하위 명령이다.
    let outcome = run_with_argv(argv(&["anemone_rs", "list-engines"]));
    assert_eq!(outcome, CliOutcome::Done(0));
}

#[test]
fn run_with_argv_dispatches_list_langs_without_touching_config_or_the_network() {
    let outcome = run_with_argv(argv(&["anemone_rs", "list-langs", "--engine", "deepl"]));
    assert_eq!(outcome, CliOutcome::Done(0));
}

#[test]
fn every_cli_engine_variant_maps_to_the_matching_translation_engine() {
    use crate::translation::TranslationEngine;

    let cases = [
        (super::Engine::EzTrans, TranslationEngine::EzTrans),
        (super::Engine::Google, TranslationEngine::Google),
        (super::Engine::DeepL, TranslationEngine::DeepL),
        (super::Engine::Papago, TranslationEngine::Papago),
        (super::Engine::Llm, TranslationEngine::Llm),
        (
            super::Engine::MysTranslater,
            TranslationEngine::MysTranslater,
        ),
        (super::Engine::Custom, TranslationEngine::Custom),
    ];
    for (cli_engine, expected) in cases {
        assert_eq!(TranslationEngine::from(cli_engine), expected);
    }
}

#[test]
fn command_definition_is_valid() {
    Cli::command().debug_assert();
}

#[test]
fn parses_existing_command_shapes() {
    assert!(Cli::try_parse_from(["anemone_rs", "translate", "테스트"]).is_ok());
    assert!(
        Cli::try_parse_from([
            "anemone_rs",
            "file-trans",
            "--in",
            "input.txt",
            "--out",
            "output.txt",
            "--format",
            "both-nl",
        ])
        .is_ok()
    );
}

#[test]
fn rejects_removed_config_commands() {
    assert!(Cli::try_parse_from(["anemone_rs", "config"]).is_err());
    assert!(Cli::try_parse_from(["anemone_rs", "config", "show"]).is_err());
    assert!(Cli::try_parse_from(["anemone_rs", "config-path"]).is_err());
    assert!(
        Cli::try_parse_from([
            "anemone_rs",
            "config",
            "set",
            "translation.engine",
            "google",
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "anemone_rs",
            "config",
            "set-secret",
            "translation.llm.api_key",
        ])
        .is_err()
    );
}

#[test]
fn rejects_removed_json_flag() {
    assert!(Cli::try_parse_from(["anemone_rs", "--json", "list-engines"]).is_err());
    assert!(Cli::try_parse_from(["anemone_rs", "list-engines", "--json"]).is_err());
}

#[test]
fn rejects_extra_arguments_and_invalid_engine() {
    assert!(Cli::try_parse_from(["anemone_rs", "list-engines", "extra"]).is_err());
    assert!(
        Cli::try_parse_from(["anemone_rs", "translate", "테스트", "--engine", "unknown",]).is_err()
    );
}

#[test]
fn accepts_custom_engine() {
    assert!(
        Cli::try_parse_from(["anemone_rs", "translate", "테스트", "--engine", "custom"]).is_ok()
    );
}

#[test]
fn translate_requires_exactly_one_input_source() {
    assert!(Cli::try_parse_from(["anemone_rs", "translate"]).is_err());
    assert!(Cli::try_parse_from(["anemone_rs", "translate", "테스트", "--stdin"]).is_err());
    assert!(Cli::try_parse_from(["anemone_rs", "translate", "--stdin"]).is_ok());
}

#[test]
fn double_dash_allows_flag_like_translation_text() {
    assert!(Cli::try_parse_from(["anemone_rs", "translate", "--", "--json"]).is_ok());
}
