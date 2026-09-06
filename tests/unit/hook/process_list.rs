use super::{Arch, ProcessEntry, deduplicate_processes};

#[test]
fn duplicate_windows_from_same_x86_process_are_collapsed() {
    let mut entries = vec![
        ProcessEntry {
            pid: 42,
            title: "Game".to_string(),
            name: "game.exe".to_string(),
            arch: Some(Arch::X86),
        },
        ProcessEntry {
            pid: 42,
            title: "Game helper".to_string(),
            name: "game.exe".to_string(),
            arch: Some(Arch::X86),
        },
    ];

    deduplicate_processes(&mut entries);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].pid, 42);
}

#[test]
fn lists_at_least_console_window_or_empty_without_panic() {
    // 환경에 따라 결과 크기가 다르지만 패닉/크래시가 없어야 한다.
    let _ = super::visible_windows();
}

#[test]
fn self_process_is_excluded() {
    let list = super::visible_windows();
    assert!(list.iter().all(|entry| entry.pid != std::process::id()));
}
