use super::*;

fn profile(name: &str, digest: Option<&str>, hook_name: &str) -> SavedHookProfile {
    SavedHookProfile {
        process_name: name.into(),
        exe_sha256: digest.map(str::to_string),
        hook_name: hook_name.into(),
        hook_code: None,
    }
}

#[test]
fn profile_lookup_prefers_hash_and_falls_back_to_name() {
    let config = HookConfig {
        saved_profiles: vec![
            profile("game.exe", Some("old"), "old-hook"),
            profile("renamed.exe", Some("new"), "new-hook"),
        ],
        ..Default::default()
    };

    assert_eq!(
        config
            .saved_profile("game.exe", Some("new"))
            .unwrap()
            .hook_name,
        "new-hook"
    );
    assert_eq!(
        config.saved_profile("GAME.EXE", None).unwrap().hook_name,
        "old-hook"
    );
    assert!(config.saved_profile("game.exe", Some("changed")).is_none());
}

#[test]
fn saving_a_profile_replaces_the_same_game() {
    let mut config = HookConfig::default();
    config.save_profile(profile("game.exe", Some("hash"), "first"));
    config.save_profile(profile("GAME.EXE", Some("hash"), "second"));

    assert_eq!(config.saved_profiles.len(), 1);
    assert_eq!(config.saved_profiles[0].hook_name, "second");
}

#[test]
fn normalization_rejects_empty_profiles_and_caps_history() {
    let mut config = HookConfig::default();
    config.saved_profiles.push(profile("", None, "invalid"));
    for index in 0..70 {
        config
            .saved_profiles
            .push(profile(&format!("game-{index}.exe"), None, "hook"));
    }

    config.normalize();

    assert_eq!(config.saved_profiles.len(), MAX_SAVED_PROFILES);
    assert_eq!(config.saved_profiles[0].process_name, "game-6.exe");
}

/// 종전 기본값 50은 글자 단위 훅의 문장을 한가운데서 끊었다. 설정 파일에 그
/// 값이 남아 있어도 원본 `flushDelay`와 같은 100까지 올린다.
#[test]
fn a_too_short_merge_window_is_raised_to_the_original_flush_delay() {
    let mut config = HookConfig {
        merge_window_ms: 50,
        ..HookConfig::default()
    };
    config.normalize();
    assert_eq!(config.merge_window_ms, 100);

    let mut longer = HookConfig {
        merge_window_ms: 400,
        ..HookConfig::default()
    };
    longer.normalize();
    assert_eq!(longer.merge_window_ms, 400);

    // 창이 지나야 문장이 나가므로, 너무 큰 값은 출력이 멈춘 것처럼 보인다.
    let mut absurd = HookConfig {
        merge_window_ms: 60_000,
        ..HookConfig::default()
    };
    absurd.normalize();
    assert_eq!(absurd.merge_window_ms, 5_000);
}
