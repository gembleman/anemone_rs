use super::*;

#[test]
fn self_process_is_native_bitness() {
    // SAFETY: 자기 자신의 pid는 항상 유효하다.
    let arch = unsafe { detect_arch(std::process::id()) }.expect("self arch");
    assert_eq!(arch, Arch::X64, "anemone은 x64 전용 빌드다");
}

#[test]
fn wait_failed_is_not_reported_as_timeout_and_keeps_win32_error() {
    let error = io::Error::from_raw_os_error(6);
    let result = classify_wait_result(WAIT_FAILED, Some(error));
    match result {
        Err(InjectError::WaitFailed(error)) => assert_eq!(error.raw_os_error(), Some(6)),
        other => panic!("expected WaitFailed, got {other:?}"),
    }
    assert!(matches!(
        classify_wait_result(WAIT_TIMEOUT, None),
        Err(InjectError::Timeout)
    ));
}

#[test]
fn wait_object_0_is_success() {
    assert!(classify_wait_result(WAIT_OBJECT_0, None).is_ok());
}

/// WaitForSingleObject 문서에 없는 반환값도 패닉하지 않고 WaitFailed로
/// 안전하게 분류해야 한다.
#[test]
fn an_unexpected_wait_status_is_reported_as_wait_failed() {
    const BOGUS_STATUS: u32 = 0x1234;
    let result = classify_wait_result(BOGUS_STATUS, None);
    match result {
        Err(InjectError::WaitFailed(error)) => {
            assert!(error.to_string().contains("1234"));
        }
        other => panic!("expected WaitFailed, got {other:?}"),
    }
}

#[test]
fn opening_a_nonexistent_process_reports_open_process_error() {
    // Windows pid는 4의 배수로 제한되므로 u32::MAX는 유효한 프로세스일 수 없다.
    let result = open_process(u32::MAX);
    assert!(matches!(result, Err(InjectError::OpenProcess(_))));
}

#[test]
fn last_win_error_reflects_the_most_recent_failure() {
    // SAFETY: OpenProcess(0, 0, u32::MAX)는 실패해 GetLastError를 갱신한다.
    let _ = unsafe { windows_sys::Win32::System::Threading::OpenProcess(0, 0, u32::MAX) };
    // last_win_error는 직전 실패의 Win32 오류를 그대로 감싼다(0이 아니어야 한다).
    assert_ne!(last_win_error().raw_os_error(), Some(0));
}
