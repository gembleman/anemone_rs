#[test]
fn trackbar_thumb_notifications_apply_the_reported_position() {
    use windows_sys::Win32::UI::Controls::{TB_ENDTRACK, TB_THUMBPOSITION, TB_THUMBTRACK};

    let wparam = 173usize << 16;
    assert_eq!(
        crate::dialogs::trackbar_thumb_position(TB_THUMBTRACK, wparam),
        Some(173)
    );
    assert_eq!(
        crate::dialogs::trackbar_thumb_position(TB_THUMBPOSITION, wparam),
        Some(173)
    );
    assert_eq!(
        crate::dialogs::trackbar_thumb_position(TB_ENDTRACK, wparam),
        None
    );
}

#[test]
fn settings_tabs_include_information_tab() {
    assert_eq!(super::TAB_INFO, 4);
    assert_eq!(super::TAB_COUNT, 5);
    assert_eq!(super::APP_VERSION, env!("CARGO_PKG_VERSION"));
    assert!(!super::APP_VERSION.is_empty());
}

/// `settings.rc`의 컨트롤 정의에서 나타나는 모든 10진수를 모은다.
///
/// 리소스 문법을 완전히 해석하지 않는다. `EDITTEXT 1004, ...`처럼 ID가 첫
/// 인자인 형태와 `LTEXT "...", 2402, ...`처럼 문자열 뒤에 오는 형태가 섞여
/// 있으므로, 콤마와 공백 양쪽으로 쪼개 숫자만 취한다. 좌표값까지 섞이지만
/// 이 집합은 "선언된 ID의 상위 집합"이면 충분하다 — 목적이 **누락 검출**이라
/// 상위 집합이어도 거짓 통과만 없으면 된다.
fn declared_control_ids(rc: &str) -> std::collections::HashSet<u16> {
    let mut ids = std::collections::HashSet::new();
    for line in rc.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        for token in line.split([',', ' ', '\t']) {
            if let Ok(value) = token.trim().parse::<u16>() {
                ids.insert(value);
            }
        }
    }
    ids
}

/// 컨트롤 ID 상수와 `settings.rc`가 어긋나면 `GetDlgItem`이 실패하고
/// `register_ids`가 그 오류를 전파해 **설정 창 전체가 열리지 않는다.**
///
/// Win32 데스크톱이 필요한 스모크 테스트는 모두 `#[ignore]`라 CI에서 돌지
/// 않으므로, 리소스 텍스트를 직접 대조해 같은 사고를 막는다.
#[test]
fn every_registered_control_id_exists_in_the_resource_script() {
    use super::ctrl_id;

    const SETTINGS_RC: &str = include_str!("../../../../resources/settings.rc");
    let declared = declared_control_ids(SETTINGS_RC);

    let groups: &[(&str, &[u16])] = &[
        ("APPEARANCE_STATIC_IDS", ctrl_id::APPEARANCE_STATIC_IDS),
        ("DISPLAY_STATIC_IDS", ctrl_id::DISPLAY_STATIC_IDS),
        ("TRANSLATION_COMMON_IDS", ctrl_id::TRANSLATION_COMMON_IDS),
        ("HOTKEYS_STATIC_IDS", ctrl_id::HOTKEYS_STATIC_IDS),
        ("INFO_STATIC_IDS", ctrl_id::INFO_STATIC_IDS),
        ("EZTRANS_STATIC_IDS", ctrl_id::EZTRANS_STATIC_IDS),
        ("DEEPL_STATIC_IDS", ctrl_id::DEEPL_STATIC_IDS),
        ("PAPAGO_STATIC_IDS", ctrl_id::PAPAGO_STATIC_IDS),
        ("LLM_STATIC_IDS", ctrl_id::LLM_STATIC_IDS),
        ("CUSTOM_STATIC_IDS", ctrl_id::CUSTOM_STATIC_IDS),
        ("APPEARANCE_IDS", super::init::APPEARANCE_IDS),
        ("DISPLAY_IDS", super::init::DISPLAY_IDS),
        ("HOTKEYS_IDS", super::init::HOTKEYS_IDS),
        ("INFO_IDS", super::init::INFO_IDS),
    ];

    let mut missing = Vec::new();
    for (name, ids) in groups {
        for &id in *ids {
            if !declared.contains(&id) {
                missing.push(format!("{name}: {id}"));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "settings.rc에 없는 컨트롤 ID가 등록되어 설정 창이 열리지 않습니다: {missing:?}"
    );
}

/// 번역 탭 컨트롤이 어느 목록에도 없으면 탭 전환 시 숨겨지지 않고, 스크롤할 때
/// 혼자 제자리에 남는다. 실제로 EzTrans 경고 라벨(2207/2208)이 엔진 그룹에만
/// 등록되어 다른 탭 위에 남아 그려지는 버그가 있었다.
///
/// `settings.rc`의 "번역 탭" 구획에 선언된 ID를 모두 훑어, 공용 목록이나 엔진
/// 그룹 중 한 곳에는 반드시 들어 있는지 확인한다.
#[test]
fn every_translation_tab_control_is_registered_for_show_hide() {
    use super::ctrl_id;

    const SETTINGS_RC: &str = include_str!("../../../../resources/settings.rc");

    // 리소스에서 번역 탭 구획만 잘라 낸다.
    let start = SETTINGS_RC
        .find("// 번역 탭")
        .expect("settings.rc에 번역 탭 구획 주석이 있어야 한다");
    let end = SETTINGS_RC[start..]
        .find("// 단축키 탭")
        .map(|offset| start + offset)
        .expect("settings.rc에 단축키 탭 구획 주석이 있어야 한다");
    let section = &SETTINGS_RC[start..end];

    // 컨트롤 ID는 첫 인자이거나 문자열 뒤 첫 숫자다. 좌표와 구분하기 위해
    // 이 다이얼로그가 실제로 쓰는 ID 대역(1200~1599, 2000~2499)만 취한다.
    let is_control_id =
        |value: u16| (1200..=1599).contains(&value) || (2000..=2499).contains(&value);
    let declared: std::collections::BTreeSet<u16> = declared_control_ids(section)
        .into_iter()
        .filter(|&id| is_control_id(id))
        .collect();
    assert!(
        declared.len() > 30,
        "번역 탭 구획 파싱이 잘못되었습니다: {declared:?}"
    );

    let mut registered = std::collections::BTreeSet::new();
    registered.extend(ctrl_id::TRANSLATION_COMMON_IDS.iter().copied());
    for ids in [
        ctrl_id::EZTRANS_STATIC_IDS,
        ctrl_id::DEEPL_STATIC_IDS,
        ctrl_id::PAPAGO_STATIC_IDS,
        ctrl_id::LLM_STATIC_IDS,
        ctrl_id::CUSTOM_STATIC_IDS,
    ] {
        registered.extend(ids.iter().copied());
    }
    registered.extend(super::init::engine_control_ids());

    let unregistered: Vec<u16> = declared.difference(&registered).copied().collect();
    assert!(
        unregistered.is_empty(),
        "번역 탭 컨트롤이 어느 목록에도 등록되지 않아 탭 전환/스크롤에서 누락됩니다: {unregistered:?}"
    );
}

#[cfg(test)]
#[path = "smoke_engine.rs"]
mod smoke_engine;

#[cfg(test)]
#[path = "smoke_scroll.rs"]
mod smoke_scroll;

#[cfg(test)]
#[path = "smoke_misc.rs"]
mod smoke_misc;
