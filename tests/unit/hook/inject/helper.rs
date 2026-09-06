use super::*;
use std::os::windows::process::ExitStatusExt;
use std::process::{ExitStatus, Output};

#[test]
fn helper_missing_file_is_reported() {
    let result = inject_via_helper(
        Path::new("Z:/존재하지않음/inject32.exe"),
        1,
        Path::new("a.dll"),
    );
    assert!(matches!(result, Err(InjectError::Helper(_))));
}

/// 항상 존재하는 시스템 유틸리티(`where.exe`)를 헬퍼인 척 실행해, 실제
/// `Command::spawn`/`output()` 경로(모의 `Output`으로는 닿지 않는 부분)까지
/// 왕복시킨다. `where.exe`는 존재하지 않는 이름을 찾지 못해 항상 실패하므로
/// 부작용이 없고 결정적이다 — 실제 인젝션 헬퍼처럼 동작하는지는 검증하지
/// 않는다.
fn where_exe() -> std::path::PathBuf {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    std::path::PathBuf::from(system_root)
        .join("System32")
        .join("where.exe")
}

#[test]
fn inject_via_helper_spawns_a_real_process_and_reports_its_failure() {
    let helper = where_exe();
    if !helper.is_file() {
        // 매우 드문 환경(System32에 where.exe가 없는 경우)에서는 건너뛴다.
        return;
    }
    // "1234"/"존재하지-않는-더미.dll"이라는 이름의 실행 파일은 PATH에 없으므로
    // where.exe는 항상 실패로 끝난다.
    let result = inject_via_helper(&helper, 1234, Path::new("존재하지-않는-더미.dll"));
    assert!(matches!(result, Err(InjectError::Helper(_))));
}

#[test]
fn uninject_via_helper_spawns_a_real_process_and_reports_its_failure() {
    let helper = where_exe();
    if !helper.is_file() {
        return;
    }
    let result = uninject_via_helper(&helper, 1234, 0xDEAD_BEEF);
    assert!(matches!(result, Err(InjectError::Helper(_))));
}

fn output(code: u32, stdout: &str, stderr: &str) -> Output {
    Output {
        status: ExitStatus::from_raw(code),
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

#[test]
fn a_successful_inject_output_parses_the_module_handle() {
    let result = parse_inject_output(&output(0, "1234\n", ""));
    assert_eq!(result.unwrap(), 1234);
}

#[test]
fn a_successful_inject_output_with_garbage_stdout_is_reported() {
    let result = parse_inject_output(&output(0, "not-a-number", ""));
    match result {
        Err(InjectError::Helper(message)) => {
            assert!(message.contains("모듈 핸들 출력이 잘못되었습니다"));
        }
        other => panic!("expected Helper error, got {other:?}"),
    }
}

#[test]
fn a_failed_inject_output_surfaces_the_stderr_detail() {
    let result = parse_inject_output(&output(1, "", "LoadLibraryW failed\n"));
    match result {
        Err(InjectError::Helper(message)) => assert_eq!(message, "LoadLibraryW failed"),
        other => panic!("expected Helper error, got {other:?}"),
    }
}

#[test]
fn a_failed_inject_output_without_stderr_falls_back_to_the_exit_code() {
    let result = parse_inject_output(&output(7, "", ""));
    match result {
        Err(InjectError::Helper(message)) => assert!(message.contains("exit code")),
        other => panic!("expected Helper error, got {other:?}"),
    }
}

#[test]
fn a_successful_uninject_output_is_ok() {
    assert!(parse_uninject_output(&output(0, "", "")).is_ok());
}

#[test]
fn a_failed_uninject_output_surfaces_the_stderr_detail() {
    let result = parse_uninject_output(&output(1, "", "FreeLibrary failed\n"));
    match result {
        Err(InjectError::Helper(message)) => assert_eq!(message, "FreeLibrary failed"),
        other => panic!("expected Helper error, got {other:?}"),
    }
}

#[test]
fn a_failed_uninject_output_without_stderr_falls_back_to_the_exit_code() {
    let result = parse_uninject_output(&output(9, "", ""));
    match result {
        Err(InjectError::Helper(message)) => {
            assert!(message.contains("uninject exit code"));
        }
        other => panic!("expected Helper error, got {other:?}"),
    }
}
