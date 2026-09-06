//! 외관, 창, 번역, 단축키, 정보 탭으로 구성된 Win32 설정 대화상자.

mod color_button;
mod ctrl_id;
mod engine_panel;
mod eztrans_path;
mod handlers;
mod handlers_dialogs;
mod handlers_input;
mod hotkeys;
mod init;
mod init_values;
mod layout;
mod model;
#[cfg_attr(not(mys_private), path = "mys_signup_stub.rs")]
mod mys_signup;
mod secret_format;

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::Controls::*,
    UI::Input::KeyboardAndMouse::{EnableWindow, VK_RETURN},
    UI::WindowsAndMessaging::*,
};

use engine_panel::EngineGroup;

use super::TBM_GETPOS;
use super::host::{DialogHost, DialogResult, HostedDialog};
use super::models::SettingsDraft;
use crate::app::action::AppActionSender;
use crate::config::Config;
use crate::win32::to_wide;
type Result<T> = windows_core::Result<T>;

/// 탭 인덱스
const TAB_APPEARANCE: usize = 0;
const TAB_DISPLAY: usize = 1;
const TAB_TRANSLATION: usize = 2;
const TAB_HOTKEYS: usize = 3;
const TAB_INFO: usize = 4;
/// 탭 개수. `tab_controls` 배열 크기와 일치해야 한다.
const TAB_COUNT: usize = 5;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 설정 대화상자
pub struct SettingsDialog {
    hwnd: HWND,
    draft: Rc<RefCell<SettingsDraft>>,
    /// 창을 닫을 때 미적용 preview를 되돌릴 마지막 적용 상태.
    last_applied: RefCell<SettingsDraft>,
    actions: Option<AppActionSender>,
    /// 각 탭에 속한 컨트롤 HWND 목록 (탭 전환 시 표시/숨김)
    tab_controls: [Vec<HWND>; TAB_COUNT],
    current_tab: usize,
    applied_dpi: u32,
    scroll_pos: i32,
    scroll_max: i32,
    has_unapplied_changes: Cell<bool>,
    /// 엔진별 컨트롤 (선택 엔진 패널 표시/숨김용)
    pub(super) engine_controls: [Vec<HWND>; EngineGroup::MysTranslater as usize + 1],
    /// 직전 수동 확인이 새 버전을 찾았는지. `true`이면 "업데이트 확인" 버튼을
    /// 다시 누르면 확인이 아니라 다운로드·적용을 요청한다.
    update_available: Cell<bool>,
    mys_signup_worker: RefCell<Option<mys_signup::SignupWorker>>,
    mys_signup_in_progress: Cell<bool>,
    mys_usage_worker: RefCell<Option<engine_panel::MysUsageWorker>>,
    mys_usage_refresh_in_progress: Cell<bool>,
    eztrans_path_worker: RefCell<Option<eztrans_path::EzTransPathWorker>>,
    eztrans_path_validation_pending: Cell<bool>,
}

pub(crate) struct SettingsInit {
    draft: Rc<RefCell<SettingsDraft>>,
    actions: Option<AppActionSender>,
}

impl HostedDialog for SettingsDialog {
    type Init = SettingsInit;
    const RESOURCE_ID: u16 = ctrl_id::DIALOG;

    fn create(hwnd: HWND, init: Self::Init) -> Result<Self> {
        let last_applied = RefCell::new(init.draft.borrow().clone());
        let mut dialog = SettingsDialog {
            hwnd,
            draft: init.draft,
            last_applied,
            actions: init.actions,
            tab_controls: [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            current_tab: TAB_APPEARANCE,
            applied_dpi: crate::dpi::dpi_for_window(hwnd),
            scroll_pos: 0,
            scroll_max: 0,
            has_unapplied_changes: Cell::new(false),
            engine_controls: std::array::from_fn(|_| Vec::new()),
            update_available: Cell::new(false),
            mys_signup_worker: RefCell::new(None),
            mys_signup_in_progress: Cell::new(false),
            mys_usage_worker: RefCell::new(None),
            mys_usage_refresh_in_progress: Cell::new(false),
            eztrans_path_worker: RefCell::new(None),
            eztrans_path_validation_pending: Cell::new(false),
        };
        dialog.initialize_controls()?;
        Ok(dialog)
    }

    /// `WM_CTLCOLORSTATIC`은 state를 빌리기 전에 처리한다. `SetWindowTextW`가
    /// 정적 컨트롤을 동기적으로 다시 그릴 때도 RefCell 대여 여부와 무관하게
    /// 탭 본문 배경색을 유지해야 하기 때문이다.
    fn handle_before_borrow(
        _hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<isize> {
        if msg != WM_CTLCOLORSTATIC {
            return None;
        }
        // SAFETY: WM_CTLCOLORSTATIC의 WPARAM은 유효한 HDC, LPARAM은 자식 HWND다.
        unsafe {
            let hdc = wparam as HDC;
            let _ = SetBkMode(hdc, TRANSPARENT as i32);
            // EzTrans 경로 경고만 붉은 글자로 그려 다른 안내 문구와 구분한다.
            let control_id = GetDlgCtrlID(lparam as HWND) as u16;
            if control_id == ctrl_id::EZTRANS_DICTIONARY_WARNING_LABEL
                || control_id == ctrl_id::EZTRANS_EHND_WARNING_LABEL
            {
                SetTextColor(hdc, 0x00_00_00_CC);
            }
            Some(GetSysColorBrush(COLOR_WINDOW) as isize)
        }
    }

    unsafe fn pretranslate_message(hwnd: HWND, msg: &MSG) -> bool {
        if msg.message != WM_KEYDOWN
            || msg.wParam != VK_RETURN as usize
            || unsafe { GetParent(msg.hwnd) } != hwnd
        {
            return false;
        }

        let control_id = unsafe { GetDlgCtrlID(msg.hwnd) } as u16;
        if !handlers::commits_on_enter(control_id) {
            return false;
        }

        // 단일 행 숫자 입력의 Enter는 현재 값을 draft/preview에 확정한다.
        // IsDialogMessageW까지 넘기면 dialog 기본 버튼 클릭으로 바뀐다.
        let command = usize::from(control_id) | ((EN_KILLFOCUS as usize) << 16);
        unsafe {
            let _ = SendMessageW(hwnd, WM_COMMAND, command, msg.hwnd as isize);
        }
        true
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> DialogResult {
        if msg == crate::dialogs::glossary::WM_GLOSSARY_APPLIED {
            self.glossary_applied();
            return DialogResult::Handled(1);
        }
        if msg == crate::dialogs::glossary::WM_EZTRANS_DICTIONARY_APPLIED {
            self.eztrans_dictionary_applied();
            return DialogResult::Handled(1);
        }
        if let Some(result) = self.handle_custom_message(msg, wparam, lparam) {
            return DialogResult::Handled(result);
        }
        match msg {
            WM_COMMAND => {
                let id = (wparam & 0xFFFF) as u16;
                let notify_code = ((wparam >> 16) & 0xFFFF) as u32;
                self.handle_command(id, notify_code);
                DialogResult::Handled(1)
            }
            // 적용하지 않은 preview는 창을 닫기 전에 되돌린다.
            WM_CLOSE => {
                self.discard_unapplied_changes();
                DialogResult::Close(1)
            }
            _ => DialogResult::Unhandled,
        }
    }

    fn applied_dpi(&mut self) -> Option<&mut u32> {
        // 설정창은 scroll 위치를 먼저 되돌려야 해서 after_dpi_changed에서 직접 처리한다.
        None
    }

    fn after_dpi_changed(&mut self, new_dpi: u32) {
        self.scroll_to(0);
        super::helpers::rescale_dialog_children_for_dpi(self.hwnd, self.applied_dpi, new_dpi);
        self.applied_dpi = new_dpi;
        self.adjust_dialog_size_for_tab(self.current_tab);
    }

    fn destroy(&mut self) {
        // 무료 API key 워커를 먼저 정리한다. 진행 중인 요청이 있으면 최대 2초
        // 기다리고, 못 끝내면 detach한 채 넘어간다 — 응답이 창이 사라진 뒤
        // 도착해도 PostMessageW가 조용히 실패할 뿐이라 안전하다.
        if let Some(worker) = self.mys_signup_worker.borrow_mut().take() {
            worker.shutdown();
        }
        if let Some(worker) = self.mys_usage_worker.borrow_mut().take() {
            worker.shutdown();
        }
        if let Some(worker) = self.eztrans_path_worker.borrow_mut().take() {
            worker.shutdown();
        }
        if let Some(actions) = &self.actions {
            actions.settings_dialog_closed();
        }
    }

    fn can_defer(msg: u32) -> bool {
        // WM_CLOSE는 host가 공통으로 재예약해 현재 preview handler의 대여가
        // 끝난 뒤 discard_unapplied_changes를 실행한다.
        msg == WM_COMMAND
            || msg == crate::dialogs::glossary::WM_GLOSSARY_APPLIED
            || msg == crate::dialogs::glossary::WM_EZTRANS_DICTIONARY_APPLIED
    }
}

impl SettingsDialog {
    /// `resources/settings.rc`의 모델리스 DIALOGEX 리소스를 연다.
    pub fn show(parent: HWND, config: Config, actions: Option<AppActionSender>) -> Result<HWND> {
        DialogHost::<Self>::show(
            parent,
            SettingsInit {
                draft: Rc::new(RefCell::new(SettingsDraft::new(config))),
                actions,
            },
        )
    }

    pub(crate) fn current_hwnd() -> Option<HWND> {
        DialogHost::<Self>::current_hwnd()
    }

    /// 주 창이 설정 요청을 처리한 뒤 실제 magnetic 상태를 반영한다.
    pub(crate) fn set_magnetic_checked(dialog_hwnd: HWND, enabled: bool) {
        DialogHost::<Self>::with_state(|dialog| {
            if dialog.hwnd == dialog_hwnd {
                dialog.draft.borrow_mut().magnetic_mode = enabled;
            }
        });
        unsafe {
            let _ = CheckDlgButton(
                dialog_hwnd,
                ctrl_id::USE_MAGNETIC as i32,
                if enabled { BST_CHECKED } else { BST_UNCHECKED },
            );
        }
    }

    pub(crate) fn set_clipboard_checked(dialog_hwnd: HWND, enabled: bool) {
        DialogHost::<Self>::with_state(|dialog| {
            if dialog.hwnd == dialog_hwnd {
                dialog.draft.borrow_mut().clipboard_watch = enabled;
            }
        });
        unsafe {
            let _ = CheckDlgButton(
                dialog_hwnd,
                ctrl_id::CLIPBOARD_WATCH as i32,
                if enabled { BST_CHECKED } else { BST_UNCHECKED },
            );
        }
    }

    /// 설정 창이 열려 있으면(정보 탭이 안 보이는 상태여도) 업데이트 상태 텍스트와
    /// 버튼 활성 상태를 갱신한다. 창이 닫혀 있으면 조용히 무시한다 — 업데이트
    /// 워커 결과는 창 수명과 독립적으로 도착할 수 있다.
    pub(crate) fn update_status_text(text: &str) {
        let Some(hwnd) = Self::current_hwnd() else {
            return;
        };
        unsafe {
            let control = GetDlgItem(hwnd, ctrl_id::UPDATE_STATUS as i32);
            if !control.is_null() {
                let wide = to_wide(text);
                let _ = SetWindowTextW(control, wide.as_ptr());
            }
        }
    }

    /// 확인이 진행 중인 동안 "업데이트 확인" 버튼을 비활성화해 중복 요청을 막는다.
    pub(crate) fn set_update_check_button_enabled(enabled: bool) {
        let Some(hwnd) = Self::current_hwnd() else {
            return;
        };
        unsafe {
            let control = GetDlgItem(hwnd, ctrl_id::UPDATE_CHECK_BTN as i32);
            if !control.is_null() {
                let _ = EnableWindow(control, if enabled { 1 } else { 0 });
            }
        }
    }

    /// 직전 수동 확인이 새 버전을 찾았는지 기록한다. "업데이트 확인" 버튼 클릭을
    /// 확인 요청과 다운로드·적용 요청 중 어느 쪽으로 해석할지 이 값으로 정한다.
    ///
    /// 같은 버튼이 두 가지 일을 하므로 레이블도 함께 바꾼다. 레이블이 "업데이트
    /// 확인"인 채로 다운로드가 시작되면 사용자는 자기가 무엇을 눌렀는지 알 수 없다.
    pub(crate) fn set_update_available(available: bool) {
        DialogHost::<Self>::with_state(|dialog| dialog.update_available.set(available));

        let Some(hwnd) = Self::current_hwnd() else {
            return;
        };
        let label = if available {
            "지금 업데이트"
        } else {
            "업데이트 확인"
        };
        unsafe {
            let control = GetDlgItem(hwnd, ctrl_id::UPDATE_CHECK_BTN as i32);
            if !control.is_null() {
                let wide = to_wide(label);
                let _ = SetWindowTextW(control, wide.as_ptr());
            }
        }
    }

    /// 레이아웃·그리기 관련 커스텀 메시지. 처리하지 않으면 `None`.
    fn handle_custom_message(
        &mut self,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            hotkeys::WM_HOTKEY_CAPTURED => {
                self.handle_hotkey_capture(wparam, lparam as u32);
                Some(0)
            }
            mys_signup::WM_MYS_SIGNUP_RESULT => {
                self.handle_mys_signup_result();
                Some(0)
            }
            engine_panel::WM_MYS_USAGE_RESULT => {
                self.handle_mys_usage_result();
                Some(0)
            }
            eztrans_path::WM_EZTRANS_PATH_RESULT => {
                self.handle_eztrans_path_result();
                Some(0)
            }
            WM_GETMINMAXINFO => {
                // SAFETY: LPARAM은 WM_GETMINMAXINFO 처리 중 유효한 MINMAXINFO 포인터다.
                unsafe {
                    let mm = &mut *(lparam as *mut MINMAXINFO);
                    let (min_width, min_height) = super::helpers::design_to_window_size(
                        self.hwnd,
                        Self::WIDTH,
                        Self::MIN_HEIGHT,
                    );
                    crate::window::set_min_track_size(mm, min_width, min_height);
                }
                Some(1)
            }
            WM_SIZE => {
                if wparam != SIZE_MINIMIZED as usize {
                    self.layout_for_current_size();
                }
                // None을 돌려 기본 처리를 남긴다. 여기서 1을 반환하면 다이얼로그
                // 매니저가 scrollbar 갱신 등 WM_SIZE 기본 동작을 건너뛴다.
                None
            }
            WM_DRAWITEM => {
                // SAFETY: lparam points to a valid DRAWITEMSTRUCT from the system.
                unsafe {
                    let dis = &*(lparam as *const DRAWITEMSTRUCT);
                    if dis.CtlType == ODT_BUTTON {
                        self.draw_color_button(dis);
                        return Some(1);
                    }
                }
                None
            }
            WM_VSCROLL => {
                self.handle_vertical_scroll(wparam);
                Some(0)
            }
            WM_MOUSEWHEEL => {
                if self.scroll_max > 0 {
                    let delta = ((wparam >> 16) & 0xffff) as u16 as i16 as i32;
                    let lines = delta / WHEEL_DELTA as i32;
                    let step = crate::dpi::scale(72, crate::dpi::dpi_for_window(self.hwnd));
                    self.scroll_to(self.scroll_pos - lines * step);
                    Some(0)
                } else {
                    None
                }
            }
            WM_HSCROLL => {
                // SAFETY: lparam contains a valid trackbar HWND from the system.
                unsafe {
                    let code = (wparam & 0xFFFF) as u32;
                    let trackbar_hwnd = lparam as HWND;

                    let id = GetDlgCtrlID(trackbar_hwnd) as u16;
                    let value = match super::trackbar_thumb_position(code, wparam) {
                        Some(value) => value,
                        None => match code {
                            TB_LINEUP | TB_LINEDOWN | TB_PAGEUP | TB_PAGEDOWN | TB_TOP
                            | TB_BOTTOM | TB_ENDTRACK => {
                                SendMessageW(trackbar_hwnd, TBM_GETPOS, 0, 0) as i32
                            }
                            _ => return Some(0),
                        },
                    };

                    self.handle_trackbar(id, value);
                }
                Some(0)
            }
            WM_NOTIFY => {
                // SAFETY: lparam points to a valid NMHDR struct from the system.
                unsafe {
                    let nmhdr = &*(lparam as *const NMHDR);
                    if nmhdr.code == TCN_SELCHANGE && {
                        let tab_hwnd = GetDlgItem(self.hwnd, ctrl_id::TAB_CONTROL as i32);
                        !tab_hwnd.is_null()
                    } {
                        let tab_hwnd = GetDlgItem(self.hwnd, ctrl_id::TAB_CONTROL as i32);
                        let sel = SendMessageW(tab_hwnd, TCM_GETCURSEL, 0, 0) as usize;
                        self.switch_tab(sel);
                    }
                }
                Some(0)
            }
            _ => None,
        }
    }
}

/// 열려 있는 설정창 상태를 빌려 검사·조작한다. GUI 테스트 전용이다.
#[cfg(test)]
pub(crate) fn with_settings_instance<R>(f: impl FnOnce(&mut SettingsDialog) -> R) -> R {
    let Some(result) = DialogHost::<SettingsDialog>::with_state_mut(f) else {
        panic!("settings instance is not open");
    };
    result
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/settings/mod.rs"]
mod tests;
