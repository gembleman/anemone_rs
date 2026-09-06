//! 탭 전환·가시성·오너드로 등 나머지 설정 대화상자 Win32 스모크 테스트.

use super::super::{
    SettingsDialog, TAB_APPEARANCE, TAB_DISPLAY, TAB_HOTKEYS, TAB_TRANSLATION, ctrl_id,
    with_settings_instance,
};

#[cfg(mys_private)]
#[path = "smoke_misc_mys.rs"]
mod mys;

/// EzTrans 경로 경고 라벨의 탭 전환 동작을 실제 창에서 확인한다.
///
/// 라벨(2207/2208)은 잘못된 경로에서만 뜨고, 번역 탭을 벗어나면 숨겨져야 한다.
/// 원래 이 라벨이 `tab_controls[TAB_TRANSLATION]`에서 누락돼 다른 탭 위에 붉은
/// 글씨로 남는 버그가 있었다.
///
/// 참고: 지금은 `switch_tab`이 `update_engine_controls`도 호출해 엔진 컨트롤을
/// 한 번 더 숨기므로, 등록 목록이 비어도 이 경로만으로는 잔상이 재현되지 않는다.
/// 목록 누락 자체는 `every_translation_tab_control_is_registered_for_show_hide`가
/// 막는다. 이 테스트가 지키는 것은 그 이중 안전망을 포함한 최종 가시성이다.
#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_eztrans_warning_labels_follow_tab_visibility() {
    use crate::config::Config;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetDesktopWindow, GetDlgItem, IsWindow, IsWindowVisible,
    };

    struct DialogGuard(HWND);

    impl Drop for DialogGuard {
        fn drop(&mut self) {
            if unsafe { IsWindow(self.0) != 0 } {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
    }

    fn control(dialog: HWND, id: u16) -> HWND {
        let hwnd = unsafe { GetDlgItem(dialog, id as i32) };
        assert!(!hwnd.is_null(), "control {id} not found");
        hwnd
    }

    let mut config = Config::default();
    config.translation.engine = "eztrans".into();
    config.translation.eztrans_dictionary_path = "C:\\없는경로\\NotJisJK.flat.bin".into();
    config.translation.eztrans_ehnd_path = "C:\\없는경로\\NotEhnd".into();

    let hwnd = SettingsDialog::show(unsafe { GetDesktopWindow() }, config, None).unwrap();
    let _dialog = DialogGuard(hwnd);

    let dll_warning = control(hwnd, ctrl_id::EZTRANS_DICTIONARY_WARNING_LABEL);
    let ehnd_warning = control(hwnd, ctrl_id::EZTRANS_EHND_WARNING_LABEL);

    // 잘못된 경로이므로 경고 문구가 실제로 채워져야 한다. 문구가 비면 라벨이
    // 보이든 말든 사용자에게는 아무 경고도 없는 것이고, 아래 가시성 검사도
    // 의미를 잃는다.
    with_settings_instance(|instance| {
        instance.switch_tab(TAB_TRANSLATION);
        instance.refresh_eztrans_path_warnings();
    });
    assert!(
        crate::dialogs::helpers::get_window_text(dll_warning)
            .contains("EzTrans 평면 사전이 아닙니다"),
        "잘못된 평면 사전 경로인데 경고 문구가 비어 있습니다"
    );
    assert!(
        crate::dialogs::helpers::get_window_text(ehnd_warning).contains("필터 폴더가 아닙니다"),
        "잘못된 Ehnd 경로인데 경고 문구가 비어 있습니다"
    );
    assert!(unsafe { IsWindowVisible(dll_warning) != 0 });
    assert!(unsafe { IsWindowVisible(ehnd_warning) != 0 });

    // 외관 탭으로 나가면 두 라벨 모두 숨겨져야 한다.
    with_settings_instance(|instance| {
        instance.switch_tab(TAB_APPEARANCE);
    });
    assert!(
        !unsafe { IsWindowVisible(dll_warning) != 0 },
        "EzTrans 사전 경고 라벨이 외관 탭 위에 남아 있습니다"
    );
    assert!(
        !unsafe { IsWindowVisible(ehnd_warning) != 0 },
        "EzTrans Ehnd 경고 라벨이 외관 탭 위에 남아 있습니다"
    );

    // 경로를 비우면 필수 설정 누락 경고가 표시되어야 한다. 라벨이 항상 켜져
    // 있는 것이 아님을 확인해 위 가시성 검사가 자명하게 통과하는 것을 막는다.
    with_settings_instance(|instance| {
        instance.switch_tab(TAB_TRANSLATION);
        {
            let mut draft = instance.draft.borrow_mut();
            draft.translation.eztrans_dictionary_path = String::new();
            draft.translation.eztrans_ehnd_path = String::new();
        }
        instance.refresh_eztrans_path_warnings();
    });
    assert!(
        crate::dialogs::helpers::get_window_text(dll_warning)
            .contains("EzTrans 평면 사전이 선택되지 않았습니다"),
        "경로가 비었는데도 사전 경고가 표시되지 않습니다"
    );
    assert!(
        crate::dialogs::helpers::get_window_text(ehnd_warning)
            .contains("EzTrans 필터 폴더가 선택되지 않았습니다"),
        "Ehnd 경로가 비었는데도 필수 설정 경고가 표시되지 않습니다"
    );
}

#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_display_tab_keeps_cache_controls_visible() {
    use crate::config::Config;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetDesktopWindow, GetDlgItem, GetWindowRect, IsWindow, IsWindowVisible,
    };

    struct DialogGuard(HWND);

    impl Drop for DialogGuard {
        fn drop(&mut self) {
            if unsafe { IsWindow(self.0) != 0 } {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
    }

    fn control(dialog: HWND, id: u16) -> HWND {
        let hwnd = unsafe { GetDlgItem(dialog, id as i32) };
        assert!(!hwnd.is_null(), "control {id} not found");
        hwnd
    }

    let hwnd =
        SettingsDialog::show(unsafe { GetDesktopWindow() }, Config::default(), None).unwrap();
    let _dialog = DialogGuard(hwnd);
    with_settings_instance(|instance| {
        instance.switch_tab(TAB_DISPLAY);
    });

    let cache_clear = control(hwnd, ctrl_id::CLIPBOARD_CACHE_CLEAR);
    let guard = control(hwnd, ctrl_id::CLIPBOARD_SOURCE_LANG_GUARD);
    let mut group_rect = Default::default();
    let mut cache_clear_rect = Default::default();
    let mut guard_rect = Default::default();
    let mut tab_rect = Default::default();
    let mut apply_rect = Default::default();
    unsafe {
        assert_ne!(GetWindowRect(control(hwnd, 2101), &mut group_rect), 0);
        assert_ne!(GetWindowRect(cache_clear, &mut cache_clear_rect), 0);
        assert_ne!(GetWindowRect(guard, &mut guard_rect), 0);
        assert_ne!(
            GetWindowRect(control(hwnd, ctrl_id::TAB_CONTROL), &mut tab_rect),
            0
        );
        assert_ne!(
            GetWindowRect(control(hwnd, ctrl_id::APPLY), &mut apply_rect),
            0
        );
    }

    assert!(unsafe { IsWindowVisible(cache_clear) != 0 });
    assert!(cache_clear_rect.bottom < group_rect.bottom);
    assert!(cache_clear_rect.bottom < tab_rect.bottom);
    assert!(group_rect.bottom < apply_rect.top);

    // 소스 언어 방어 옵션은 그룹의 마지막 줄이다. 그룹박스 높이를 함께 늘리지
    // 않으면 테두리를 뚫고 나간다.
    assert!(unsafe { IsWindowVisible(guard) != 0 });
    assert!(guard_rect.top >= cache_clear_rect.bottom);
    assert!(guard_rect.bottom < group_rect.bottom);
    assert!(guard_rect.bottom < tab_rect.bottom);
}

/// 탭 전환 중 동기 repaint가 `DialogHost`의 state 대여에 재진입하면 owner-draw
/// 색상 버튼의 `WM_DRAWITEM`이 버려져 버튼이 빈 회색으로 남는다.
#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_owner_draw_color_survives_tab_switch() {
    use crate::config::Config;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC, UpdateWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetDesktopWindow, GetDlgItem, IsWindow,
    };

    struct DialogGuard(HWND);

    impl Drop for DialogGuard {
        fn drop(&mut self) {
            if unsafe { IsWindow(self.0) != 0 } {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
    }

    let config = Config {
        background_color: 0xff_12_34_56,
        ..Config::default()
    };
    let hwnd = SettingsDialog::show(unsafe { GetDesktopWindow() }, config, None).unwrap();
    let _dialog = DialogGuard(hwnd);

    with_settings_instance(|instance| {
        instance.switch_tab(TAB_DISPLAY);
        instance.switch_tab(TAB_APPEARANCE);
    });

    let button = unsafe { GetDlgItem(hwnd, ctrl_id::BACKGROUND_COLOR as i32) };
    assert!(!button.is_null(), "color button not found");
    unsafe {
        // 비동기로 예약된 paint를 state 대여가 끝난 뒤 처리한다.
        assert_ne!(UpdateWindow(button), 0, "owner-draw repaint");
        let hdc = GetDC(button);
        assert!(!hdc.is_null());
        let pixel = GetPixel(hdc, 5, 5);
        let _ = ReleaseDC(button, hdc);
        assert_eq!(
            pixel, 0x00_56_34_12,
            "탭 전환 후 배경색 owner-draw 버튼이 다시 그려지지 않았습니다"
        );
    }
}

#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_hotkeys_tab_layout_smoke() {
    use crate::config::Config;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetDesktopWindow, GetDlgItem, GetWindowRect, IsWindow, IsWindowVisible,
        SendMessageW, WM_COMMAND,
    };

    struct DialogGuard(HWND);

    impl Drop for DialogGuard {
        fn drop(&mut self) {
            if unsafe { IsWindow(self.0) != 0 } {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
    }

    fn control(dialog: HWND, id: u16) -> HWND {
        let hwnd = unsafe { GetDlgItem(dialog, id as i32) };
        assert!(!hwnd.is_null(), "control {id} not found");
        hwnd
    }

    let mut config = Config::default();
    config.hotkeys.toggle_window = "F8".parse().unwrap();
    let hwnd = SettingsDialog::show(unsafe { GetDesktopWindow() }, config, None).unwrap();
    let _dialog = DialogGuard(hwnd);
    with_settings_instance(|instance| {
        instance.switch_tab(TAB_HOTKEYS);
    });

    let mut list_rect = Default::default();
    let mut reset_rect = Default::default();
    let mut tab_rect = Default::default();
    let mut apply_rect = Default::default();
    unsafe {
        assert_ne!(
            GetWindowRect(control(hwnd, ctrl_id::HOTKEYS_LIST), &mut list_rect),
            0
        );
        assert_ne!(
            GetWindowRect(control(hwnd, ctrl_id::HOTKEYS_RESET), &mut reset_rect),
            0
        );
        assert_ne!(
            GetWindowRect(control(hwnd, ctrl_id::TAB_CONTROL), &mut tab_rect),
            0
        );
        assert_ne!(
            GetWindowRect(control(hwnd, ctrl_id::APPLY), &mut apply_rect),
            0
        );
    }

    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::HOTKEYS_LIST)) != 0 });
    assert!(unsafe { IsWindowVisible(control(hwnd, ctrl_id::HOTKEYS_RESET)) != 0 });
    assert!(list_rect.bottom <= tab_rect.bottom);
    assert!(reset_rect.top > list_rect.bottom);
    assert!(reset_rect.bottom <= tab_rect.bottom);
    assert!(list_rect.bottom < apply_rect.top);

    let reset = control(hwnd, ctrl_id::HOTKEYS_RESET);
    unsafe {
        let _ = SendMessageW(
            hwnd,
            WM_COMMAND,
            usize::from(ctrl_id::HOTKEYS_RESET),
            reset as LPARAM,
        );
    }
    with_settings_instance(|instance| {
        assert_eq!(instance.draft.borrow().hotkeys, Default::default());
        assert!(instance.has_unapplied_changes.get());
    });
}
