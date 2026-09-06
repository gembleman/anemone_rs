use super::*;

#[test]
fn digests_the_running_executable() {
    let digest = compute_exe_digest(std::process::id()).expect("자기 자신의 exe는 읽을 수 있다");
    assert_eq!(digest.len(), 64);
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

#[test]
fn nonexistent_pid_yields_no_digest() {
    // Windows pid는 4의 배수로 제한되므로 u32::MAX는 유효하지 않다.
    // "조회/해시 실패 시 None" 계약 확인.
    assert!(compute_exe_digest(u32::MAX).is_none());
}

#[test]
fn session_identity_snapshot_round_trips() {
    set_session_identity(Some(ProcessIdentity {
        pid: 42,
        name: "game.exe".to_string(),
        sha256: None,
    }));
    assert_eq!(
        session_identity().map(|identity| identity.name),
        Some("game.exe".to_string())
    );
    set_session_identity(None);
    assert!(session_identity().is_none());
}

#[test]
fn session_active_snapshot_round_trips() {
    set_session_active(true);
    assert!(session_active());
    set_session_active(false);
    assert!(!session_active());
}

#[test]
fn arch_folder_and_label_match_the_bitness() {
    assert_eq!(Arch::X64.folder(), "x64");
    assert_eq!(Arch::X64.label(), "x64");
    assert_eq!(Arch::X86.folder(), "x86");
    assert_eq!(Arch::X86.label(), "x86");
}

/// hook_dir/dll_path/inject32_helper_path는 모두 `current_exe()`의 부모
/// 디렉터리를 기준으로 조립된다 — 실제 exe 옆에 배치된 `hook/<arch>/`
/// 폴더 규칙이 어긋나면 인젝션이 조용히 실패하므로 조립 규칙 자체를 고정한다.
#[test]
fn dll_and_helper_paths_are_rooted_next_to_the_current_executable() {
    let exe_dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    assert_eq!(hook_dir(Arch::X64), exe_dir.join("hook").join("x64"));
    assert_eq!(hook_dir(Arch::X86), exe_dir.join("hook").join("x86"));
    assert_eq!(
        dll_path(Arch::X64),
        exe_dir.join("hook").join("x64").join("lunahook_rs64.dll")
    );
    assert_eq!(
        dll_path(Arch::X86),
        exe_dir.join("hook").join("x86").join("lunahook_rs32.dll")
    );
    assert_eq!(
        inject32_helper_path(),
        exe_dir
            .join("hook")
            .join("x86")
            .join("anemone_inject32.exe")
    );
}
