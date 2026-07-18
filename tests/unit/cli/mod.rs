use clap::{CommandFactory, Parser, error::ErrorKind};

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
            "--json",
        ])
        .is_ok()
    );
    assert!(
        Cli::try_parse_from([
            "anemone_rs",
            "config",
            "set",
            "translation.engine",
            "google",
        ])
        .is_ok()
    );
}

#[test]
fn json_without_command_is_an_error_instead_of_a_panic() {
    let error = match Cli::try_parse_from(["anemone_rs", "--json"]) {
        Ok(_) => panic!("명령 없는 --json을 허용하면 안 됨"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ErrorKind::MissingSubcommand);
}

#[test]
fn rejects_extra_arguments_and_invalid_engine() {
    assert!(Cli::try_parse_from(["anemone_rs", "list-engines", "extra"]).is_err());
    assert!(
        Cli::try_parse_from(["anemone_rs", "translate", "테스트", "--engine", "unknown",]).is_err()
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
