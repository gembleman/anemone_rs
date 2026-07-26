//! 외관, 창, 번역, 단축키, 정보 탭으로 구성된 Win32 설정 대화상자.

mod ctrl_id;
mod handlers;
mod hotkeys;
mod init;
mod model;

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, UI::Controls::*,
        UI::Input::KeyboardAndMouse::EnableWindow, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::TBM_GETPOS;
use super::host::{DialogHost, DialogResult, HostedDialog};
use super::models::SettingsDraft;
use crate::app::action::AppActionSender;
use crate::config::{Config, TextAlign};
use crate::translation::{TranslationEngine, lang_utils};
use crate::win32::to_wide;

/// 탭 인덱스
const TAB_APPEARANCE: usize = 0;
const TAB_DISPLAY: usize = 1;
const TAB_TRANSLATION: usize = 2;
const TAB_HOTKEYS: usize = 3;
const TAB_INFO: usize = 4;
/// 탭 개수. `tab_controls` 배열 크기와 일치해야 한다.
const TAB_COUNT: usize = 5;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 선택된 엔진 패널만 표시하기 위한 컨트롤 그룹.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EngineGroup {
    EzTrans = 0,
    DeepL = 1,
    Papago = 2,
    Llm = 3,
    Custom = 4,
}

pub(super) fn mask_secret(secret: &str) -> String {
    if secret.is_empty() {
        return String::new();
    }
    if secret.chars().count() <= 4 {
        return "••••".to_string();
    }
    let suffix: String = secret
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("••••{suffix}")
}

pub(super) fn format_deepl_key(secret: &str) -> String {
    let tier = crate::translation::DeepLApiTier::from_api_key(secret);
    format!("[{}] {}", tier.display_name(), mask_secret(secret))
}

/// `EnumChildWindows` 콜백: 자식 핸들을 `LPARAM`이 가리키는 Vec에 모은다.
///
/// 열거 중에는 창을 옮기지 않는다. `SetWindowPos`를 콜백 안에서 호출하면
/// 열거 순서가 흐트러져 일부 컨트롤을 건너뛸 수 있기 때문이다.
unsafe extern "system" fn collect_direct_child(child: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam은 호출부가 넘긴 유효한 Vec<HWND> 포인터이며, 열거가
    // 끝날 때까지 살아 있다.
    unsafe {
        if let Some(children) = (lparam.0 as *mut Vec<HWND>).as_mut() {
            children.push(child);
        }
    }
    TRUE
}

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
    pub(super) engine_controls: [Vec<HWND>; 5],
    /// 직전 수동 확인이 새 버전을 찾았는지. `true`이면 "업데이트 확인" 버튼을
    /// 다시 누르면 확인이 아니라 다운로드·적용을 요청한다.
    update_available: Cell<bool>,
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
            engine_controls: [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            update_available: Cell::new(false),
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
            let hdc = HDC(wparam.0 as *mut _);
            let _ = SetBkMode(hdc, TRANSPARENT);
            // EzTrans 경로 경고만 붉은 글자로 그려 다른 안내 문구와 구분한다.
            let control_id = GetDlgCtrlID(HWND(lparam.0 as *mut _)) as u16;
            if control_id == ctrl_id::EZTRANS_DLL_WARNING_LABEL
                || control_id == ctrl_id::EZTRANS_DAT_WARNING_LABEL
            {
                SetTextColor(hdc, COLORREF(0x00_00_00_CC));
            }
            Some(GetSysColorBrush(COLOR_WINDOW).0 as isize)
        }
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> DialogResult {
        if msg == crate::dialogs::glossary::WM_GLOSSARY_APPLIED {
            self.glossary_applied();
            return DialogResult::Handled(LRESULT(1));
        }
        if msg == crate::dialogs::glossary::WM_EZTRANS_DICTIONARY_APPLIED {
            self.eztrans_dictionary_applied();
            return DialogResult::Handled(LRESULT(1));
        }
        if let Some(result) = self.handle_custom_message(msg, wparam, lparam) {
            return DialogResult::Handled(result);
        }
        match msg {
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as u16;
                let notify_code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                self.handle_command(id, notify_code);
                DialogResult::Handled(LRESULT(1))
            }
            // 적용하지 않은 preview는 창을 닫기 전에 되돌린다.
            WM_CLOSE => {
                self.discard_unapplied_changes();
                DialogResult::Close(LRESULT(1))
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
    // 리소스 컨트롤의 좌우 여백을 같게 맞춘 96 DPI 디자인 폭.
    // 높이는 선택한 탭에 따라 동적으로 바뀐다.
    const WIDTH: i32 = 492;
    // 가로 컨트롤은 고정 배치이므로 잘리지 않는 최소 client 폭을 유지한다.
    const MIN_HEIGHT: i32 = 180;

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
            if let Ok(control) = GetDlgItem(Some(hwnd), ctrl_id::UPDATE_STATUS as i32) {
                let _ = SetWindowTextW(control, &HSTRING::from(text));
            }
        }
    }

    /// 확인이 진행 중인 동안 "업데이트 확인" 버튼을 비활성화해 중복 요청을 막는다.
    pub(crate) fn set_update_check_button_enabled(enabled: bool) {
        let Some(hwnd) = Self::current_hwnd() else {
            return;
        };
        unsafe {
            if let Ok(control) = GetDlgItem(Some(hwnd), ctrl_id::UPDATE_CHECK_BTN as i32) {
                let _ = EnableWindow(control, enabled);
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
            if let Ok(control) = GetDlgItem(Some(hwnd), ctrl_id::UPDATE_CHECK_BTN as i32) {
                let _ = SetWindowTextW(control, &HSTRING::from(label));
            }
        }
    }

    /// ctrl_id로부터 미리보기 색상(ARGB)을 찾는다
    fn color_for_button(&self, id: u16) -> Option<u32> {
        use crate::config::{ColorType, TextType};
        let cfg = self.draft.borrow();
        let argb = match id {
            ctrl_id::BACKGROUND_COLOR => cfg.background_color,
            ctrl_id::BORDER_COLOR => cfg.border_color,
            ctrl_id::NAME_COLOR => cfg.get_text_color(TextType::Name, ColorType::Primary),
            ctrl_id::NAME_OUTLINE1 => cfg.get_text_color(TextType::Name, ColorType::Outline1),
            ctrl_id::NAME_OUTLINE2 => cfg.get_text_color(TextType::Name, ColorType::Outline2),
            ctrl_id::NAME_SHADOW_COLOR => cfg.get_text_color(TextType::Name, ColorType::Shadow),
            ctrl_id::ORG_COLOR => cfg.get_text_color(TextType::Original, ColorType::Primary),
            ctrl_id::ORG_OUTLINE1 => cfg.get_text_color(TextType::Original, ColorType::Outline1),
            ctrl_id::ORG_OUTLINE2 => cfg.get_text_color(TextType::Original, ColorType::Outline2),
            ctrl_id::ORG_SHADOW_COLOR => cfg.get_text_color(TextType::Original, ColorType::Shadow),
            ctrl_id::TRANS_COLOR => cfg.get_text_color(TextType::Translation, ColorType::Primary),
            ctrl_id::TRANS_OUTLINE1 => {
                cfg.get_text_color(TextType::Translation, ColorType::Outline1)
            }
            ctrl_id::TRANS_OUTLINE2 => {
                cfg.get_text_color(TextType::Translation, ColorType::Outline2)
            }
            ctrl_id::TRANS_SHADOW_COLOR => {
                cfg.get_text_color(TextType::Translation, ColorType::Shadow)
            }
            _ => return None,
        };
        Some(argb)
    }

    /// 색상 버튼에 swatch와 text를 그린다.
    fn draw_color_button(&self, dis: &DRAWITEMSTRUCT) {
        let id = dis.CtlID as u16;
        let Some(argb) = self.color_for_button(id) else {
            return;
        };
        // SAFETY: hDC and rcItem are valid for the duration of WM_DRAWITEM.
        unsafe {
            let hdc = dis.hDC;
            let rc = dis.rcItem;

            // 배경: 시스템 버튼 면색
            let bg_brush = GetSysColorBrush(COLOR_BTNFACE);
            let _ = FillRect(hdc, &rc, bg_brush);

            // 테두리 (눌림 / 포커스 상태에 따라 다르게)
            let pressed = (dis.itemState.0 & ODS_SELECTED.0) != 0;
            if pressed {
                let _ = DrawEdge(hdc, &mut { rc }, EDGE_SUNKEN, BF_RECT);
            } else {
                let _ = DrawEdge(hdc, &mut { rc }, EDGE_RAISED, BF_RECT);
            }

            // 색상 스왓치 (왼쪽 1/3 영역)
            let pad = 4;
            let swatch_w = ((rc.right - rc.left - pad * 3) / 3).max(12);
            let swatch_rc = RECT {
                left: rc.left + pad,
                top: rc.top + pad,
                right: rc.left + pad + swatch_w,
                bottom: rc.bottom - pad,
            };
            // ARGB → COLORREF (0x00BBGGRR)
            let r = (argb >> 16) & 0xFF;
            let g = (argb >> 8) & 0xFF;
            let b = argb & 0xFF;
            let colorref = COLORREF((b << 16) | (g << 8) | r);
            let brush = CreateSolidBrush(colorref);
            let _ = FillRect(hdc, &swatch_rc, brush);
            // 스왓치 외곽선
            let pen_color = COLORREF(0x00808080);
            let pen = CreatePen(PS_SOLID, 1, pen_color);
            let old_pen = SelectObject(hdc, pen.into());
            let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
            let _ = Rectangle(
                hdc,
                swatch_rc.left,
                swatch_rc.top,
                swatch_rc.right,
                swatch_rc.bottom,
            );
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_brush);
            let _ = DeleteObject(pen.into());
            let _ = DeleteObject(brush.into());

            // 텍스트 (오른쪽 2/3 영역)
            let mut text_rc = RECT {
                left: swatch_rc.right + pad,
                top: rc.top,
                right: rc.right - pad,
                bottom: rc.bottom,
            };
            let mut buf = [0u16; 64];
            let len = GetWindowTextW(dis.hwndItem, &mut buf);
            if len > 0 {
                let _ = SetBkMode(hdc, TRANSPARENT);
                let _ = DrawTextW(
                    hdc,
                    &mut buf[..len as usize],
                    &mut text_rc,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
            }

            // 포커스 사각형
            if (dis.itemState.0 & ODS_FOCUS.0) != 0 {
                let _ = DrawFocusRect(hdc, &rc);
            }
        }
    }

    /// 색상 변경 후 해당 버튼만 강제 다시 그리기
    pub(super) fn invalidate_color_button(&self, id: u16) {
        Self::invalidate_color_button_for(self.hwnd, id);
    }

    fn invalidate_color_button_for(hwnd: HWND, id: u16) {
        // SAFETY: hwnd is a valid settings window; GetDlgItem returns its child control.
        unsafe {
            if let Ok(h) = GetDlgItem(Some(hwnd), id as i32) {
                let _ = InvalidateRect(Some(h), None, true);
            }
        }
    }

    /// 탭 전환: 현재 탭 컨트롤 숨기고 새 탭 컨트롤 표시
    fn switch_tab(&mut self, new_tab: usize) {
        if new_tab == self.current_tab || new_tab >= self.tab_controls.len() {
            return;
        }
        // 번역 탭은 선택된 엔진의 컨트롤만 보여야 하므로, 먼저 그룹박스를 엔진 높이에
        // 맞춘 뒤 표시한다. 전부 SW_SHOW 했다가 되숨기면 비활성 엔진 패널이 깜빡인다.
        let engine = if new_tab == TAB_TRANSLATION {
            self.draft.borrow().translation.get_engine().ok()
        } else {
            None
        };
        if let Some(engine) = engine {
            self.resize_translation_group(engine);
        }

        // SAFETY: All HWNDs in tab_controls are valid child window handles.
        unsafe {
            for &hwnd in &self.tab_controls[self.current_tab] {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            for &hwnd in &self.tab_controls[new_tab] {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
        }
        self.current_tab = new_tab;
        // 엔진 컨트롤 중에는 tab_controls에 없는 것(EzTrans 경고 라벨 등)이 있어
        // 위의 hide 루프로는 숨겨지지 않는다. 번역 탭을 벗어날 때도 반드시 호출한다.
        if let Ok(engine) = self.draft.borrow().translation.get_engine() {
            self.update_engine_controls(engine);
        }
        self.adjust_dialog_size_for_tab(new_tab);
    }

    fn translation_height_for_engine(engine: TranslationEngine) -> i32 {
        match engine {
            TranslationEngine::EzTrans => 312,
            TranslationEngine::Google | TranslationEngine::Papago | TranslationEngine::Custom => {
                245
            }
            TranslationEngine::DeepL => 365,
            TranslationEngine::Llm => 490,
        }
    }

    /// 선택된 엔진의 전용 입력을 감싸도록 번역 설정 그룹박스 높이를 반환한다.
    fn translation_group_height_for_engine(engine: TranslationEngine) -> i32 {
        match engine {
            TranslationEngine::DeepL => 136,
            TranslationEngine::Llm => 204,
            TranslationEngine::EzTrans => 110,
            TranslationEngine::Google | TranslationEngine::Papago | TranslationEngine::Custom => 69,
        }
    }

    fn resize_translation_group(&self, engine: TranslationEngine) {
        let Ok(group) = (unsafe { GetDlgItem(Some(self.hwnd), ctrl_id::TRANSLATION_GROUP as i32) })
        else {
            return;
        };
        let mut current = RECT::default();
        if unsafe { GetWindowRect(group, &mut current) }.is_err() {
            return;
        }
        let mut size = RECT {
            bottom: Self::translation_group_height_for_engine(engine),
            ..Default::default()
        };
        unsafe {
            let _ = MapDialogRect(self.hwnd, &mut size);
            let _ = SetWindowPos(
                group,
                None,
                0,
                0,
                current.right - current.left,
                size.bottom,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    fn target_height_for_tab(&self, tab: usize) -> i32 {
        match tab {
            TAB_APPEARANCE => 505,
            TAB_DISPLAY => 330,
            TAB_TRANSLATION => self
                .draft
                .borrow()
                .translation
                .get_engine()
                .map(Self::translation_height_for_engine)
                .unwrap_or(245),
            TAB_HOTKEYS => 360,
            // 업데이트와 스페셜 땡스 UI가 들어간 정보 그룹박스 높이에 맞춘다.
            TAB_INFO => 487,
            _ => 505,
        }
    }

    /// 탭에 따라 다이얼로그 클라이언트 높이를 조정 (빈 공간 최소화)
    fn adjust_dialog_size_for_tab(&mut self, tab: usize) {
        // 각 탭의 마지막 group 아래에 닫기 button이 오도록 높이를 잡는다.
        let target_height = self.target_height_for_tab(tab);
        // SAFETY: self.hwnd is valid. SetWindowPos uses valid parameters.
        unsafe {
            // scroll_max는 아직 이전 탭 기준이라 scroll_to(0)은 clamp에 걸려
            // 오프셋을 되돌리지 못할 수 있다. 자식 위치를 직접 원점으로 되돌린다.
            self.reset_scroll_offset();
            let monitor = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let work = if GetMonitorInfoW(monitor, &mut info).as_bool() {
                info.rcWork
            } else {
                RECT {
                    left: 0,
                    top: 0,
                    right: GetSystemMetrics(SM_CXSCREEN),
                    bottom: GetSystemMetrics(SM_CYSCREEN),
                }
            };
            let work_width = work.right - work.left;
            let work_height = work.bottom - work.top;

            // Client 디자인 크기를 title/border를 포함한 window 크기로 바꾼다.
            let (_, desired_height) =
                super::helpers::design_to_window_size(self.hwnd, Self::WIDTH, target_height);
            let needs_scroll = desired_height > work_height;
            let style = GetWindowLongPtrW(self.hwnd, GWL_STYLE) as u32;
            let new_style = if needs_scroll {
                style | WS_VSCROLL.0
            } else {
                style & !WS_VSCROLL.0
            };
            if style != new_style {
                let _ = SetWindowLongPtrW(self.hwnd, GWL_STYLE, new_style as _);
            }

            // Scrollbar가 client 폭을 줄이지 않도록 전체 크기를 다시 계산한다.
            let (desired_width, desired_height) =
                super::helpers::design_to_window_size(self.hwnd, Self::WIDTH, target_height);
            let win_width = desired_width.min(work_width);
            let win_height = desired_height.min(work_height);
            let mut rect = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut rect);
            let x = if win_width >= work_width {
                work.left
            } else {
                rect.left.clamp(work.left, work.right - win_width)
            };
            let y = if win_height >= work_height {
                work.top
            } else {
                rect.top.clamp(work.top, work.bottom - win_height)
            };
            // SWP_NOCOPYBITS: 크기가 바뀔 때 이전 탭의 픽셀이 새 위치로 복사되어
            // 잔상으로 남는 것을 막는다.
            let _ = SetWindowPos(
                self.hwnd,
                None,
                x,
                y,
                win_width,
                win_height,
                SWP_NOZORDER | SWP_FRAMECHANGED | SWP_NOCOPYBITS,
            );
        }
        self.layout_for_current_size();
        self.redraw_all();
    }

    /// 탭 전환·크기 조정 후 부모와 모든 자식을 다시 그리도록 예약한다.
    ///
    /// 이 함수는 dialog state를 가변 대여한 상태에서 호출된다. `RDW_UPDATENOW`로
    /// 동기 paint를 강제하면 owner-draw 버튼의 `WM_DRAWITEM`이 재진입하고,
    /// `DialogHost`가 같은 state를 다시 대여하지 못해 색상 버튼이 빈 채로 남는다.
    fn redraw_all(&self) {
        // SAFETY: self.hwnd는 설정창 수명 동안 유효한 핸들이다.
        unsafe {
            let _ = RedrawWindow(
                Some(self.hwnd),
                None,
                None,
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
            );
        }
    }

    /// 사용자가 테두리를 끌어 바꾼 client 크기에 tab, 하단 button, scrollbar를 맞춘다.
    fn layout_for_current_size(&mut self) {
        // SAFETY: self.hwnd와 자식 컨트롤은 설정창 수명 동안 유효하다.
        unsafe {
            let mut client = RECT::default();
            if GetClientRect(self.hwnd, &mut client).is_err() {
                return;
            }
            let height = (client.bottom - client.top).max(1);
            let dpi = crate::dpi::dpi_for_window(self.hwnd);
            let s = |v: i32| crate::dpi::scale(v, dpi);
            let content_height = s(self.target_height_for_tab(self.current_tab));
            let scroll_max = (content_height - height).max(0);
            let new_scroll_pos = self.scroll_pos.clamp(0, scroll_max);

            // 수동으로 창을 낮춘 경우에도 표준 세로 scrollbar를 즉시 표시한다.
            let _ = ShowScrollBar(self.hwnd, SB_VERT, scroll_max > 0);
            let _ = GetClientRect(self.hwnd, &mut client);
            let width = (client.right - client.left).max(1);
            let height = (client.bottom - client.top).max(1);

            if new_scroll_pos != self.scroll_pos {
                let delta = self.scroll_pos - new_scroll_pos;
                self.offset_scroll_children(delta);
            }
            self.scroll_pos = new_scroll_pos;
            self.scroll_max = scroll_max;

            // Tab은 가로로 창을 채우고, 세로로는 기존 내용 또는 viewport 중 큰 쪽을 쓴다.
            if let Ok(tab_hwnd) = GetDlgItem(Some(self.hwnd), ctrl_id::TAB_CONTROL as i32) {
                let _ = SetWindowPos(
                    tab_hwnd,
                    None,
                    s(5),
                    s(5) - new_scroll_pos,
                    (width - s(10)).max(1),
                    (content_height.max(height) - s(70)).max(1),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }

            // 내용이 모두 보이면 하단에 고정하고, 스크롤 중이면 기존처럼 내용 끝에 둔다.
            let button_y = if scroll_max == 0 {
                height - s(65)
            } else {
                content_height - s(65) - new_scroll_pos
            };
            let mut button_right = width - s(15);
            for id in [ctrl_id::CLOSE, ctrl_id::APPLY] {
                if let Ok(button) = GetDlgItem(Some(self.hwnd), id as i32) {
                    let mut button_rect = RECT::default();
                    if GetWindowRect(button, &mut button_rect).is_err() {
                        continue;
                    }
                    let button_width = button_rect.right - button_rect.left;
                    let button_height = button_rect.bottom - button_rect.top;
                    button_right -= button_width;
                    let _ = SetWindowPos(
                        button,
                        None,
                        button_right,
                        button_y,
                        button_width,
                        button_height,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    button_right -= s(5);
                }
            }

            let scroll_info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                nMin: 0,
                nMax: content_height.saturating_sub(1),
                nPage: height as u32,
                nPos: new_scroll_pos,
                ..Default::default()
            };
            SetScrollInfo(self.hwnd, SB_VERT, &scroll_info, true);
        }
    }

    /// 탭 전환 직전에 스크롤 오프셋을 맨 위로 되돌린다.
    ///
    /// `scroll_to(0)`과 달리 `scroll_max`(아직 이전 탭 값)를 참조하지 않으므로,
    /// 새 탭의 높이를 계산하기 전에도 자식 위치를 확실히 원점으로 맞춘다.
    fn reset_scroll_offset(&mut self) {
        if self.scroll_pos == 0 {
            return;
        }
        // SAFETY: self.hwnd와 그 자식 컨트롤은 설정창 수명 동안 유효하다.
        unsafe {
            self.offset_scroll_children(self.scroll_pos);
        }
        self.scroll_pos = 0;
    }

    /// 낮은 해상도에서 잘린 설정 내용을 세로로 이동한다.
    fn scroll_to(&mut self, position: i32) {
        let new_pos = position.clamp(0, self.scroll_max);
        if new_pos == self.scroll_pos {
            return;
        }

        // SAFETY: self.hwnd와 그 자식 컨트롤은 설정창 수명 동안 유효하다.
        unsafe {
            let delta = self.scroll_pos - new_pos;
            self.offset_scroll_children(delta);
            self.scroll_pos = new_pos;
            let scroll_info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_POS,
                nPos: new_pos,
                ..Default::default()
            };
            SetScrollInfo(self.hwnd, SB_VERT, &scroll_info, true);
            let _ = InvalidateRect(Some(self.hwnd), None, true);
            let _ = UpdateWindow(self.hwnd);
        }
    }

    /// 화면 밖의 항목도 빠지지 않도록 모든 직접 자식을 이동한다.
    unsafe fn offset_scroll_children(&self, delta: i32) {
        unsafe fn offset(parent: HWND, child: HWND, delta: i32) {
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(child, &mut rect) }.is_err() {
                return;
            }
            let mut top_left = POINT {
                x: rect.left,
                y: rect.top,
            };
            unsafe {
                let _ = ScreenToClient(parent, &mut top_left);
                let _ = SetWindowPos(
                    child,
                    None,
                    top_left.x,
                    top_left.y + delta,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }

        // 등록 목록(tab_controls/engine_controls)을 순회하면 양쪽에 중복 등록된
        // 컨트롤이 두 번 이동하고, 어느 쪽에도 없는 컨트롤(예: EzTrans 경고 라벨)은
        // 아예 이동하지 않는다. 실제 자식 창을 열거해 하나씩만 정확히 옮긴다.
        unsafe {
            let mut children: Vec<HWND> = Vec::new();
            let _ = EnumChildWindows(
                Some(self.hwnd),
                Some(collect_direct_child),
                LPARAM(&mut children as *mut Vec<HWND> as isize),
            );
            for child in children {
                // 탭 컨트롤의 손자(자식의 자식)는 부모를 따라 함께 움직인다.
                if GetParent(child).ok() == Some(self.hwnd) {
                    offset(self.hwnd, child, delta);
                }
            }
        }
    }

    fn handle_vertical_scroll(&mut self, wparam: WPARAM) {
        if self.scroll_max == 0 {
            return;
        }

        let command = (wparam.0 & 0xffff) as i32;
        let dpi = crate::dpi::dpi_for_window(self.hwnd);
        let line = crate::dpi::scale(24, dpi);
        let mut client = RECT::default();
        // SAFETY: self.hwnd는 유효한 설정창 핸들이다.
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
        }
        let page = (client.bottom - client.top - line).max(line);
        let target = match command {
            value if value == SB_LINEUP.0 => self.scroll_pos - line,
            value if value == SB_LINEDOWN.0 => self.scroll_pos + line,
            value if value == SB_PAGEUP.0 => self.scroll_pos - page,
            value if value == SB_PAGEDOWN.0 => self.scroll_pos + page,
            value if value == SB_TOP.0 => 0,
            value if value == SB_BOTTOM.0 => self.scroll_max,
            value if value == SB_THUMBTRACK.0 || value == SB_THUMBPOSITION.0 => {
                let mut info = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_TRACKPOS,
                    ..Default::default()
                };
                // SAFETY: info는 쓰기 가능한 SCROLLINFO이고 hwnd에 SB_VERT가 있다.
                if unsafe { GetScrollInfo(self.hwnd, SB_VERT, &mut info) }.is_ok() {
                    info.nTrackPos
                } else {
                    ((wparam.0 >> 16) & 0xffff) as i32
                }
            }
            _ => return,
        };
        self.scroll_to(target);
    }

    fn engine_group(engine: TranslationEngine) -> Option<EngineGroup> {
        match engine {
            TranslationEngine::EzTrans => Some(EngineGroup::EzTrans),
            TranslationEngine::DeepL => Some(EngineGroup::DeepL),
            TranslationEngine::Papago => Some(EngineGroup::Papago),
            TranslationEngine::Llm => Some(EngineGroup::Llm),
            TranslationEngine::Custom => Some(EngineGroup::Custom),
            TranslationEngine::Google => None,
        }
    }

    /// 번역 탭에서는 선택된 엔진의 전용 컨트롤만 표시한다.
    fn update_engine_controls(&self, engine: TranslationEngine) {
        let active = Self::engine_group(engine).map(|group| group as usize);
        let translation_tab_visible = self.current_tab == TAB_TRANSLATION;
        // SAFETY: HWNDs in engine_controls are valid child controls.
        unsafe {
            for (idx, group) in self.engine_controls.iter().enumerate() {
                let visible = translation_tab_visible && active == Some(idx);
                for &h in group {
                    let _ = EnableWindow(h, visible);
                    let _ = ShowWindow(h, if visible { SW_SHOW } else { SW_HIDE });
                }
            }
        }
    }

    /// 현재 엔진에 맞춰 소스/타겟 언어 콤보 항목을 갱신
    pub(super) fn refresh_language_combos(&self, engine: TranslationEngine) {
        // SAFETY: self.hwnd is valid; GetDlgItem returns valid combobox handles.
        unsafe {
            let src_combo = GetDlgItem(Some(self.hwnd), ctrl_id::TRANS_SOURCE_LANG as i32);
            let tgt_combo = GetDlgItem(Some(self.hwnd), ctrl_id::TRANS_TARGET_LANG as i32);
            let (Ok(src), Ok(tgt)) = (src_combo, tgt_combo) else {
                return;
            };

            let _ = SendMessageW(src, CB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
            for &lang in engine.supported_source_languages() {
                let w = to_wide(lang_utils::to_korean_name(lang));
                let _ = SendMessageW(
                    src,
                    CB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(w.as_ptr() as isize)),
                );
            }
            let src_sel = match self.draft.borrow().translation.source_lang_index(engine) {
                Ok(index) => index,
                Err(error) => {
                    tracing::error!("번역 언어 설정 오류: {error}");
                    return;
                }
            };
            let _ = SendMessageW(src, CB_SETCURSEL, Some(WPARAM(src_sel)), Some(LPARAM(0)));

            let source = engine
                .supported_source_languages()
                .get(src_sel)
                .copied()
                .or_else(|| engine.supported_source_languages().first().copied());
            let Some(source) = source else {
                return;
            };
            let targets = engine.supported_targets_for(source);

            let _ = SendMessageW(tgt, CB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
            for &lang in &targets {
                let w = to_wide(lang_utils::to_korean_name(lang));
                let _ = SendMessageW(
                    tgt,
                    CB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(w.as_ptr() as isize)),
                );
            }
            let configured_target = self.draft.borrow().translation.get_target_language().ok();
            let tgt_sel = configured_target
                .and_then(|target| targets.iter().position(|&language| language == target))
                .unwrap_or(0);
            let _ = SendMessageW(tgt, CB_SETCURSEL, Some(WPARAM(tgt_sel)), Some(LPARAM(0)));
        }
    }

    /// 엔진별 패널, 언어 콤보, 번역 탭 높이를 함께 갱신한다.
    pub(super) fn apply_engine_state(&mut self, engine: TranslationEngine) {
        self.resize_translation_group(engine);
        self.update_engine_controls(engine);
        self.refresh_language_combos(engine);
        if self.current_tab == TAB_TRANSLATION {
            self.adjust_dialog_size_for_tab(TAB_TRANSLATION);
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
                self.handle_hotkey_capture(wparam.0, lparam.0 as u32);
                Some(LRESULT(0))
            }
            WM_GETMINMAXINFO => {
                // SAFETY: LPARAM은 WM_GETMINMAXINFO 처리 중 유효한 MINMAXINFO 포인터다.
                unsafe {
                    let mm = &mut *(lparam.0 as *mut MINMAXINFO);
                    let (min_width, min_height) = super::helpers::design_to_window_size(
                        self.hwnd,
                        Self::WIDTH,
                        Self::MIN_HEIGHT,
                    );
                    crate::window::set_min_track_size(mm, min_width, min_height);
                }
                Some(LRESULT(1))
            }
            WM_SIZE => {
                if wparam.0 != SIZE_MINIMIZED as usize {
                    self.layout_for_current_size();
                }
                // None을 돌려 기본 처리를 남긴다. 여기서 1을 반환하면 다이얼로그
                // 매니저가 scrollbar 갱신 등 WM_SIZE 기본 동작을 건너뛴다.
                None
            }
            WM_DRAWITEM => {
                // SAFETY: lparam points to a valid DRAWITEMSTRUCT from the system.
                unsafe {
                    let dis = &*(lparam.0 as *const DRAWITEMSTRUCT);
                    if dis.CtlType == ODT_BUTTON {
                        self.draw_color_button(dis);
                        return Some(LRESULT(1));
                    }
                }
                None
            }
            WM_VSCROLL => {
                self.handle_vertical_scroll(wparam);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                if self.scroll_max > 0 {
                    let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
                    let lines = delta / WHEEL_DELTA as i32;
                    let step = crate::dpi::scale(72, crate::dpi::dpi_for_window(self.hwnd));
                    self.scroll_to(self.scroll_pos - lines * step);
                    Some(LRESULT(0))
                } else {
                    None
                }
            }
            WM_HSCROLL => {
                // SAFETY: lparam contains a valid trackbar HWND from the system.
                unsafe {
                    let code = (wparam.0 & 0xFFFF) as u32;
                    let trackbar_hwnd = HWND(lparam.0 as *mut _);

                    let id = GetDlgCtrlID(trackbar_hwnd) as u16;
                    let value = match super::trackbar_thumb_position(code, wparam.0) {
                        Some(value) => value,
                        None => match code {
                            TB_LINEUP | TB_LINEDOWN | TB_PAGEUP | TB_PAGEDOWN | TB_TOP
                            | TB_BOTTOM | TB_ENDTRACK => {
                                SendMessageW(
                                    trackbar_hwnd,
                                    TBM_GETPOS,
                                    Some(WPARAM(0)),
                                    Some(LPARAM(0)),
                                )
                                .0 as i32
                            }
                            _ => return Some(LRESULT(0)),
                        },
                    };

                    self.handle_trackbar(id, value);
                }
                Some(LRESULT(0))
            }
            WM_NOTIFY => {
                // SAFETY: lparam points to a valid NMHDR struct from the system.
                unsafe {
                    let nmhdr = &*(lparam.0 as *const NMHDR);
                    if nmhdr.code == TCN_SELCHANGE
                        && let Ok(tab_hwnd) =
                            GetDlgItem(Some(self.hwnd), ctrl_id::TAB_CONTROL as i32)
                    {
                        let sel =
                            SendMessageW(tab_hwnd, TCM_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)))
                                .0 as usize;
                        self.switch_tab(sel);
                    }
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

/// 열려 있는 설정창 상태를 빌려 검사·조작한다. GUI 테스트 전용이다.
#[cfg(test)]
pub(crate) fn with_settings_instance<R>(f: impl FnOnce(&mut SettingsDialog) -> R) -> R {
    DialogHost::<SettingsDialog>::with_state_mut(f).expect("settings instance")
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/settings/mod.rs"]
mod tests;
