use clap::{CommandFactory, Parser};

use super::Cli;

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
