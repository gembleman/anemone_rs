//! 외관, 창, 번역 탭으로 구성된 Win32 설정 대화상자.

mod ctrl_id;
mod handlers;
mod init;

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW, UI::Controls::*,
        UI::Input::KeyboardAndMouse::EnableWindow, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::helpers::{
    center_dialog_on_monitor, register_resource_dialog, show_dialog_window,
    unregister_resource_dialog,
};
use crate::config::{Config, TextAlign};
use crate::constants::TBM_GETPOS_VAL;
use crate::define_dialog_instance;
use crate::translation::{TranslationEngine, lang_utils};
use crate::util::to_wide;

/// 설정 변경 콜백 타입
pub type SettingsChangeCallback = Box<dyn Fn(&Config)>;

/// 탭 인덱스
const TAB_APPEARANCE: usize = 0;
const TAB_DISPLAY: usize = 1;
const TAB_TRANSLATION: usize = 2;

/// 엔진별 컨트롤 그룹 (활성/비활성 토글용)
#[derive(Clone, Copy)]
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

fn should_persist_trackbar(code: u32) -> bool {
    code == TB_ENDTRACK
}

/// 설정 대화상자
pub struct SettingsDialog {
    hwnd: HWND,
    config: Rc<RefCell<Config>>,
    main_hwnd: HWND,
    on_change: Option<SettingsChangeCallback>,
    /// 각 탭에 속한 컨트롤 HWND 목록 (탭 전환 시 표시/숨김)
    tab_controls: [Vec<HWND>; 3],
    current_tab: usize,
    applied_dpi: u32,
    scroll_pos: i32,
    scroll_max: i32,
    pending_disk_save: Cell<bool>,
    /// 엔진별 컨트롤 (EnableWindow 토글용)
    pub(super) engine_controls: [Vec<HWND>; 5],
}

define_dialog_instance!(SETTINGS_INSTANCE: SettingsDialog);

struct PendingSettings {
    parent: HWND,
    config: Rc<RefCell<Config>>,
    on_change: Option<SettingsChangeCallback>,
}

thread_local! {
    static SETTINGS_PENDING: RefCell<Option<PendingSettings>> = const { RefCell::new(None) };
    static SETTINGS_INIT_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// `resources/settings.rc`에서 생성된 모델리스 다이얼로그의 메시지 콜백.
unsafe extern "system" fn settings_dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    unsafe {
        if msg == WM_INITDIALOG {
            let pending = SETTINGS_PENDING.with(|slot| slot.borrow_mut().take());
            let Some(PendingSettings {
                parent,
                config,
                on_change,
            }) = pending
            else {
                SETTINGS_INIT_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some("설정창 초기화 인자가 없습니다".to_string());
                });
                return 0;
            };

            let dialog = Rc::new(RefCell::new(SettingsDialog {
                hwnd,
                config,
                main_hwnd: parent,
                on_change,
                tab_controls: [Vec::new(), Vec::new(), Vec::new()],
                current_tab: TAB_APPEARANCE,
                applied_dpi: crate::dpi::dpi_for_window(hwnd),
                scroll_pos: 0,
                scroll_max: 0,
                pending_disk_save: Cell::new(false),
                engine_controls: [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            }));
            SETTINGS_INSTANCE.with(|slot| {
                *slot.borrow_mut() = Some(dialog.clone());
            });

            if let Err(error) = dialog.borrow_mut().initialize_controls() {
                SETTINGS_INSTANCE.with(|slot| {
                    slot.borrow_mut().take();
                });
                SETTINGS_INIT_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some(error.to_string());
                });
                return 0;
            }
            register_resource_dialog(hwnd);
            return 1;
        }

        let instance = SETTINGS_INSTANCE.with(|slot| {
            let Ok(guard) = slot.try_borrow() else {
                return None;
            };
            guard.clone()
        });
        let Some(dialog) = instance else {
            return 0;
        };

        if msg == WM_DPICHANGED {
            let mut can_flush = false;
            if let Ok(mut dialog) = dialog.try_borrow_mut() {
                dialog.handle_dpi_changed(wparam, lparam);
                can_flush = true;
            } else {
                super::helpers::defer_dialog_dpi_change(hwnd, wparam, lparam);
            }
            if can_flush {
                super::helpers::flush_deferred_dialog_messages(hwnd);
            }
            return 1;
        }

        if msg == crate::dialogs::glossary::WM_GLOSSARY_APPLIED {
            if let Ok(dialog) = dialog.try_borrow() {
                dialog.refresh_glossary_count();
                drop(dialog);
                super::helpers::flush_deferred_dialog_messages(hwnd);
            } else {
                super::helpers::defer_dialog_message(hwnd, msg, wparam, lparam);
            }
            return 1;
        }

        if let Ok(mut dialog) = dialog.try_borrow_mut()
            && let Some(result) = dialog.handle_message(msg, wparam, lparam)
        {
            drop(dialog);
            super::helpers::flush_deferred_dialog_messages(hwnd);
            return result.0;
        }

        let mut can_flush = false;
        let result = match msg {
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as u16;
                let notify_code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                if let Ok(mut dialog) = dialog.try_borrow_mut() {
                    dialog.handle_command(id, notify_code);
                    can_flush = true;
                } else {
                    super::helpers::defer_dialog_message(hwnd, msg, wparam, lparam);
                }
                1
            }
            WM_CLOSE => {
                match dialog.try_borrow() {
                    Ok(dialog) => {
                        let should_close = dialog.persist_pending_changes();
                        drop(dialog);
                        can_flush = true;
                        if should_close {
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                    Err(_) => super::helpers::defer_dialog_message(hwnd, msg, wparam, lparam),
                }
                1
            }
            WM_DESTROY => {
                if let Ok(dialog) = dialog.try_borrow() {
                    let _ = dialog.persist_pending_changes();
                }
                unregister_resource_dialog(hwnd);
                SETTINGS_INSTANCE.with(|slot| {
                    if let Ok(mut guard) = slot.try_borrow_mut() {
                        *guard = None;
                    }
                });
                1
            }
            _ => 0,
        };
        if can_flush {
            super::helpers::flush_deferred_dialog_messages(hwnd);
        }
        result
    }
}

impl SettingsDialog {
    // 리소스의 96 DPI 디자인 폭. 높이는 선택한 탭에 따라 동적으로 바뀐다.
    const WIDTH: i32 = 485;
    // 가로 컨트롤은 고정 배치이므로 잘리지 않는 최소 client 폭을 유지한다.
    const MIN_HEIGHT: i32 = 180;

    /// `resources/settings.rc`의 모델리스 DIALOGEX 리소스를 연다.
    pub fn show(
        parent: HWND,
        config: Rc<RefCell<Config>>,
        on_change: Option<SettingsChangeCallback>,
    ) -> Result<HWND> {
        if let Some(hwnd) = Self::current_hwnd() {
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
            return Ok(hwnd);
        }
        // SAFETY: None은 현재 프로세스 모듈을 뜻한다.
        let instance = unsafe { GetModuleHandleW(None)? };
        SETTINGS_INIT_ERROR.with(|slot| {
            slot.borrow_mut().take();
        });
        SETTINGS_PENDING.with(|slot| {
            *slot.borrow_mut() = Some(PendingSettings {
                parent,
                config,
                on_change,
            });
        });

        // SAFETY: 리소스 ID는 빌드 시 실행 파일에 포함되고, 콜백은 DLGPROC ABI를
        // 따른다. 초기화 인자는 UI 스레드의 SETTINGS_PENDING에서 한 번만 꺼낸다.
        let result = unsafe {
            CreateDialogParamW(
                Some(instance.into()),
                PCWSTR(ctrl_id::DIALOG as usize as *const u16),
                Some(parent),
                Some(settings_dialog_proc),
                LPARAM(0),
            )
        };

        let hwnd = match result {
            Ok(hwnd) => hwnd,
            Err(error) => {
                SETTINGS_PENDING.with(|slot| {
                    slot.borrow_mut().take();
                });
                SETTINGS_INIT_ERROR.with(|slot| {
                    slot.borrow_mut().take();
                });
                return Err(error);
            }
        };

        if let Some(message) = SETTINGS_INIT_ERROR.with(|slot| slot.borrow_mut().take()) {
            // SAFETY: CreateDialogParamW가 반환한 유효한 모델리스 다이얼로그.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(Error::new(E_FAIL, message));
        }

        // 부모가 있는 모니터의 작업 영역 중앙에 배치한다.
        unsafe {
            center_dialog_on_monitor(hwnd, parent);
            show_dialog_window(hwnd);
        }
        Ok(hwnd)
    }

    pub(crate) fn current_hwnd() -> Option<HWND> {
        let hwnd = SETTINGS_INSTANCE.with(|slot| {
            slot.borrow()
                .as_ref()
                .and_then(|dialog| dialog.try_borrow().ok().map(|dialog| dialog.hwnd))
        })?;
        unsafe { IsWindow(Some(hwnd)).as_bool().then_some(hwnd) }
    }

    /// 주 창이 설정 요청을 처리한 뒤 실제 magnetic 상태를 반영한다.
    pub(crate) fn set_magnetic_checked(dialog_hwnd: HWND, enabled: bool) {
        unsafe {
            let _ = CheckDlgButton(
                dialog_hwnd,
                ctrl_id::USE_MAGNETIC as i32,
                if enabled { BST_CHECKED } else { BST_UNCHECKED },
            );
        }
    }

    pub(crate) fn set_clipboard_checked(dialog_hwnd: HWND, enabled: bool) {
        unsafe {
            let _ = CheckDlgButton(
                dialog_hwnd,
                ctrl_id::CLIPBOARD_WATCH as i32,
                if enabled { BST_CHECKED } else { BST_UNCHECKED },
            );
        }
    }

    /// ctrl_id로부터 미리보기 색상(ARGB)을 찾는다
    fn color_for_button(&self, id: u16) -> Option<u32> {
        use crate::config::{ColorType, TextType};
        let cfg = self.config.borrow();
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
        // SAFETY: self.hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            if let Ok(h) = GetDlgItem(Some(self.hwnd), id as i32) {
                let _ = InvalidateRect(Some(h), None, true);
            }
        }
    }

    fn handle_dpi_changed(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let new_dpi = (wparam.0 & 0xffff) as u32;
        self.scroll_to(0);
        super::helpers::rescale_dialog_children_for_dpi(self.hwnd, self.applied_dpi, new_dpi);
        self.applied_dpi = new_dpi;

        if lparam.0 != 0 {
            // SAFETY: WM_DPICHANGED의 LPARAM은 메시지 처리 동안 유효한 RECT 포인터다.
            unsafe {
                let rect = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
        self.adjust_dialog_size_for_tab(self.current_tab);
    }

    /// 탭 전환: 현재 탭 컨트롤 숨기고 새 탭 컨트롤 표시
    fn switch_tab(&mut self, new_tab: usize) {
        if new_tab == self.current_tab || new_tab >= self.tab_controls.len() {
            return;
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
        self.adjust_dialog_size_for_tab(new_tab);
    }

    fn target_height_for_tab(tab: usize) -> i32 {
        match tab {
            TAB_APPEARANCE => 505,
            TAB_DISPLAY => 305,
            TAB_TRANSLATION => 1180,
            _ => 505,
        }
    }

    /// 탭에 따라 다이얼로그 클라이언트 높이를 조정 (빈 공간 최소화)
    fn adjust_dialog_size_for_tab(&mut self, tab: usize) {
        // 각 탭의 마지막 group 아래에 닫기 button이 오도록 높이를 잡는다.
        let target_height = Self::target_height_for_tab(tab);
        // SAFETY: self.hwnd is valid. SetWindowPos uses valid parameters.
        unsafe {
            self.scroll_to(0);
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
            let _ = SetWindowPos(
                self.hwnd,
                None,
                x,
                y,
                win_width,
                win_height,
                SWP_NOZORDER | SWP_FRAMECHANGED,
            );
        }
        self.layout_for_current_size();
    }

    /// 사용자가 테두리를 끌어 바꾼 client 크기에 tab, 닫기 button, scrollbar를 맞춘다.
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
            let content_height = s(Self::target_height_for_tab(self.current_tab));
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
            if let Ok(close_hwnd) = GetDlgItem(Some(self.hwnd), ctrl_id::CLOSE as i32) {
                let mut close_rect = RECT::default();
                if GetWindowRect(close_hwnd, &mut close_rect).is_ok() {
                    let close_width = close_rect.right - close_rect.left;
                    let close_height = close_rect.bottom - close_rect.top;
                    let close_y = if scroll_max == 0 {
                        height - s(65)
                    } else {
                        content_height - s(65) - new_scroll_pos
                    };
                    let _ = SetWindowPos(
                        close_hwnd,
                        None,
                        width - s(15) - close_width,
                        close_y,
                        close_width,
                        close_height,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
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

        unsafe {
            if let Ok(tab) = GetDlgItem(Some(self.hwnd), ctrl_id::TAB_CONTROL as i32) {
                offset(self.hwnd, tab, delta);
            }
            for &child in self.tab_controls.iter().flatten() {
                offset(self.hwnd, child, delta);
            }
            if let Ok(close) = GetDlgItem(Some(self.hwnd), ctrl_id::CLOSE as i32) {
                offset(self.hwnd, close, delta);
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

    /// 엔진별 컨트롤 enable 상태 갱신
    fn update_engine_enable(&self, engine: TranslationEngine) {
        let active = match engine {
            TranslationEngine::EzTrans => EngineGroup::EzTrans as usize,
            TranslationEngine::DeepL => EngineGroup::DeepL as usize,
            TranslationEngine::Papago => EngineGroup::Papago as usize,
            TranslationEngine::Llm => EngineGroup::Llm as usize,
            TranslationEngine::Custom => EngineGroup::Custom as usize,
            // Google은 별도 입력란이 없으므로 전부 비활성
            TranslationEngine::Google => usize::MAX,
        };
        // SAFETY: HWNDs in engine_controls are valid child controls.
        unsafe {
            for (idx, group) in self.engine_controls.iter().enumerate() {
                let enable = idx == active;
                for &h in group {
                    let _ = EnableWindow(h, enable);
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
            let src_sel = match self.config.borrow().translation.source_lang_index(engine) {
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
            let configured_target = self.config.borrow().translation.get_target_language().ok();
            let tgt_sel = configured_target
                .and_then(|target| targets.iter().position(|&language| language == target))
                .unwrap_or(0);
            let _ = SendMessageW(tgt, CB_SETCURSEL, Some(WPARAM(tgt_sel)), Some(LPARAM(0)));
        }
    }

    /// 엔진별 enable 상태와 언어 콤보를 동시에 갱신 (엔진 변경 시 호출)
    pub(super) fn apply_engine_state(&self, engine: TranslationEngine) {
        self.update_engine_enable(engine);
        self.refresh_language_combos(engine);
    }

    /// 커스텀 메시지 핸들러
    fn handle_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
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
                Some(LRESULT(1))
            }
            WM_CTLCOLORSTATIC => {
                // Tab 본문과 child control 배경을 모두 COLOR_WINDOW로 맞춘다.
                // SAFETY: wparam은 OS가 전달한 유효 HDC다.
                unsafe {
                    let hdc = HDC(wparam.0 as *mut _);
                    let _ = SetBkMode(hdc, TRANSPARENT);
                    let brush = GetSysColorBrush(COLOR_WINDOW);
                    Some(LRESULT(brush.0 as isize))
                }
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
                    let value = match code {
                        TB_THUMBTRACK => ((wparam.0 >> 16) & 0xFFFF) as i32,
                        TB_LINEUP | TB_LINEDOWN | TB_PAGEUP | TB_PAGEDOWN | TB_TOP | TB_BOTTOM
                        | TB_ENDTRACK => {
                            SendMessageW(
                                trackbar_hwnd,
                                TBM_GETPOS_VAL,
                                Some(WPARAM(0)),
                                Some(LPARAM(0)),
                            )
                            .0 as i32
                        }
                        _ => return Some(LRESULT(0)),
                    };

                    self.handle_trackbar(id, value);
                    if should_persist_trackbar(code) {
                        self.persist_pending_changes();
                    }
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

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/settings/mod.rs"]
mod tests;
