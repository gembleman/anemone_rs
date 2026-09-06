//! 설정 대화상자 스크롤 동작의 Win32 스모크 테스트.
//!
//! 스크롤이 모든 자식 컨트롤을 정확히 한 번씩 옮기는지, 오프셋 초기화가
//! 원래 위치를 복원하는지 확인한다.

use super::super::{SettingsDialog, TAB_HOTKEYS, TAB_TRANSLATION, ctrl_id, with_settings_instance};

/// 스크롤은 `EnumChildWindows`로 실제 자식을 훑어 각 컨트롤을 한 번씩만 옮긴다.
/// 등록 목록을 순회하던 이전 구현에서는 두 목록에 중복 등록된 컨트롤이 두 배로
/// 밀려나고, 어느 목록에도 없는 컨트롤은 제자리에 남았다.
///
/// 스크롤바가 실제로 생기는지는 모니터 작업영역 크기에 달렸으므로, `scroll_max`를
/// 직접 지정해 해상도와 무관하게 이동 로직만 검증한다.
#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_scrolling_moves_every_child_exactly_once() {
    use crate::config::Config;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetDesktopWindow, GetDlgItem, GetWindowRect, IsWindow,
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

    fn top_of(dialog: HWND, id: u16) -> i32 {
        let mut rect = Default::default();
        unsafe {
            assert_ne!(GetWindowRect(control(dialog, id), &mut rect), 0);
        }
        rect.top
    }

    let mut config = Config::default();
    config.translation.engine = "eztrans".into();
    let hwnd = SettingsDialog::show(unsafe { GetDesktopWindow() }, config, None).unwrap();
    let _dialog = DialogGuard(hwnd);

    // 번역 탭에는 공용 컨트롤과 엔진 컨트롤이 섞여 있다. 중복 등록/누락이
    // 있었다면 이 둘의 이동량이 서로 달라진다.
    let probes = [
        ctrl_id::TRANS_ENGINE,                     // 공용(tab_controls에만)
        ctrl_id::TRANSLATION_GROUP,                // 공용 그룹박스
        ctrl_id::EZTRANS_DICTIONARY_EDIT,          // 엔진 컨트롤(양쪽에 등록)
        ctrl_id::EZTRANS_DICTIONARY_WARNING_LABEL, // 과거에 누락되었던 컨트롤
        ctrl_id::APPLY,                            // 탭에 속하지 않는 하단 버튼
    ];

    with_settings_instance(|instance| {
        instance.switch_tab(TAB_TRANSLATION);
    });

    let before: Vec<i32> = probes.iter().map(|&id| top_of(hwnd, id)).collect();

    const DELTA: i32 = 40;
    with_settings_instance(|instance| {
        instance.scroll_max = 200;
        instance.scroll_to(DELTA);
    });

    let after: Vec<i32> = probes.iter().map(|&id| top_of(hwnd, id)).collect();
    for ((&id, &was), &now) in probes.iter().zip(&before).zip(&after) {
        assert_eq!(
            was - now,
            DELTA,
            "컨트롤 {id}이(가) {}px 이동했습니다(기대 {DELTA}px). \
             중복 등록이면 2배, 누락이면 0px가 됩니다.",
            was - now
        );
    }

    // 되돌리면 정확히 원래 위치여야 한다. 왕복 오차가 쌓이면 탭을 오갈 때마다
    // 레이아웃이 조금씩 밀린다.
    with_settings_instance(|instance| {
        instance.scroll_to(0);
    });
    let restored: Vec<i32> = probes.iter().map(|&id| top_of(hwnd, id)).collect();
    assert_eq!(
        restored, before,
        "스크롤 왕복 후 위치가 원래대로 돌아오지 않았습니다"
    );
}

/// `reset_scroll_offset`은 스크롤된 자식을 원점으로 되돌리고 `scroll_pos`를 0으로
/// 만들어야 한다.
///
/// 탭 전환 시 이 함수가 필요한 이유는 저해상도 경로다. `adjust_dialog_size_for_tab`은
/// 새 탭 기준으로 `scroll_max`를 다시 계산하며 `scroll_pos`를 clamp하는데, 새 탭도
/// 여전히 스크롤이 필요할 만큼 화면이 작으면 clamp가 걸리지 않아 오프셋이 남는다.
/// 그 조건은 모니터 작업영역에 의존해 일반 해상도에서는 재현되지 않으므로, 창 전환
/// 대신 함수의 계약을 직접 검증한다.
#[test]
#[ignore = "requires a Win32 desktop and embedded dialog resources"]
fn win32_reset_scroll_offset_restores_child_positions() {
    use crate::config::Config;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetDesktopWindow, GetDlgItem, GetWindowRect, IsWindow,
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

    fn top_of(dialog: HWND, id: u16) -> i32 {
        let mut rect = Default::default();
        unsafe {
            let control = GetDlgItem(dialog, id as i32);
            assert!(!control.is_null(), "control {id} not found");
            assert_ne!(GetWindowRect(control, &mut rect), 0);
        }
        rect.top
    }

    let hwnd =
        SettingsDialog::show(unsafe { GetDesktopWindow() }, Config::default(), None).unwrap();
    let _dialog = DialogGuard(hwnd);

    with_settings_instance(|instance| {
        instance.switch_tab(TAB_HOTKEYS);
    });
    let baseline = top_of(hwnd, ctrl_id::HOTKEYS_LIST);

    with_settings_instance(|instance| {
        instance.scroll_max = 200;
        instance.scroll_to(60);
    });
    assert_eq!(
        baseline - top_of(hwnd, ctrl_id::HOTKEYS_LIST),
        60,
        "스크롤이 적용되지 않아 복원 검사가 무의미합니다"
    );

    with_settings_instance(|instance| {
        instance.reset_scroll_offset();
        assert_eq!(instance.scroll_pos, 0);
    });
    assert_eq!(
        top_of(hwnd, ctrl_id::HOTKEYS_LIST),
        baseline,
        "reset_scroll_offset이 자식 컨트롤을 원점으로 되돌리지 않았습니다"
    );
}
