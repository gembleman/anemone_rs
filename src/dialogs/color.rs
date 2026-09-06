//! Alpha control과 실시간 callback을 추가한 `CHOOSECOLOR` dialog.

use std::cell::RefCell;
use std::mem::zeroed;

use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
    UI::Controls::Dialogs::*, UI::Controls::*, UI::WindowsAndMessaging::*,
};

use super::TBM_GETPOS;

const COLOR_RED_EDIT: u16 = 0x2C2;
const COLOR_GREEN_EDIT: u16 = 0x2C3;
const COLOR_BLUE_EDIT: u16 = 0x2C4;

// 알파 채널 컨트롤 ID
const IDC_ALPHA_TRACKBAR: u16 = 10001;
const IDC_ALPHA_EDIT: u16 = 10002;

/// 색상 대화상자 결과 (ARGB)
#[derive(Debug, Clone, Copy)]
pub struct ColorResult {
    pub argb: u32,
}

impl ColorResult {
    pub fn from_colorref(colorref: u32, alpha: u8) -> Self {
        let r = (colorref & 0xFF) as u8;
        let g = ((colorref >> 8) & 0xFF) as u8;
        let b = ((colorref >> 16) & 0xFF) as u8;
        Self {
            argb: ((alpha as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
        }
    }
}

/// 색상 변경 콜백 타입
pub type ColorChangeCallback = Box<dyn Fn(u32)>;

/// 색상 대화상자 설정
pub struct ColorDialogConfig {
    /// 초기 색상 (ARGB)
    pub initial_color: u32,
    /// 색상 변경 시 실시간 콜백 (선택적)
    pub on_color_change: Option<ColorChangeCallback>,
    /// WS_EX_NOACTIVATE 스타일 적용 여부
    pub no_activate: bool,
    /// 불투명도 조절 컨트롤 표시 여부
    pub show_alpha: bool,
}

impl Default for ColorDialogConfig {
    fn default() -> Self {
        Self {
            initial_color: 0xFF000000, // 불투명 검정
            on_color_change: None,
            no_activate: true,
            show_alpha: true,
        }
    }
}

/// 훅 프로시저용 컨텍스트
struct HookContext {
    alpha: i32,
    callback: Option<ColorChangeCallback>,
    no_activate: bool,
    show_alpha: bool,
}

thread_local! {
    static HOOK_CONTEXT: RefCell<Option<HookContext>> = const { RefCell::new(None) };
    static CUSTOM_COLORS: RefCell<[COLORREF; 16]> = const { RefCell::new([0xFFFFFF; 16]) };
}

pub struct ColorDialog;

impl ColorDialog {
    /// 색상 선택 대화상자 표시
    pub fn show(hwnd: HWND, config: ColorDialogConfig) -> Option<ColorResult> {
        // SAFETY: 호출자가 유효한 hwnd를 제공한다.
        unsafe { Self::show_impl(hwnd, config) }
    }

    unsafe fn show_impl(hwnd: HWND, config: ColorDialogConfig) -> Option<ColorResult> {
        // SAFETY: 구조체 크기, owner, color buffer와 hook pointer가 모두 유효하다.
        unsafe {
            let alpha = ((config.initial_color >> 24) & 0xFF) as i32;
            let r = ((config.initial_color >> 16) & 0xFF) as u8;
            let g = ((config.initial_color >> 8) & 0xFF) as u8;
            let b = (config.initial_color & 0xFF) as u8;
            let initial_colorref = ((b as u32) << 16) | ((g as u32) << 8) | (r as u32);

            // 훅 컨텍스트 설정
            HOOK_CONTEXT.with(|ctx| {
                if let Ok(mut guard) = ctx.try_borrow_mut() {
                    *guard = Some(HookContext {
                        alpha,
                        callback: config.on_color_change,
                        no_activate: config.no_activate,
                        show_alpha: config.show_alpha,
                    });
                }
            });

            let mut custom_colors =
                CUSTOM_COLORS.with(|c| c.try_borrow().map_or([0xFFFFFF; 16], |g| *g));

            let mut cc: CHOOSECOLORW = zeroed();
            cc.lStructSize = size_of::<CHOOSECOLORW>() as u32;
            cc.hwndOwner = hwnd;
            cc.lpCustColors = custom_colors.as_mut_ptr();
            cc.rgbResult = initial_colorref;
            cc.lCustData = alpha as isize;
            cc.Flags = CC_FULLOPEN | CC_RGBINIT | CC_ENABLEHOOK;
            cc.lpfnHook = Some(Self::hook_proc);

            let result = ChooseColorW(&mut cc);

            // 커스텀 색상 저장
            CUSTOM_COLORS.with(|c| {
                if let Ok(mut guard) = c.try_borrow_mut() {
                    *guard = custom_colors;
                }
            });

            // 컨텍스트에서 최종 알파값 가져오기
            let final_alpha = HOOK_CONTEXT.with(|ctx| {
                ctx.try_borrow()
                    .ok()
                    .and_then(|g| g.as_ref().map(|c| c.alpha as u8))
                    .unwrap_or(alpha as u8)
            });

            // 컨텍스트 정리
            HOOK_CONTEXT.with(|ctx| {
                if let Ok(mut guard) = ctx.try_borrow_mut() {
                    *guard = None;
                }
            });

            if result != 0 {
                Some(ColorResult::from_colorref(cc.rgbResult, final_alpha))
            } else {
                None
            }
        }
    }

    /// 다이얼로그 DPI 기준으로 96-DPI 디자인 좌표/크기를 스케일링한다.
    fn scale_for_dpi(value: i32, hwnd: HWND) -> i32 {
        crate::dpi::scale_for_window(value, hwnd)
    }

    /// 다이얼로그에서 ARGB 색상 읽기
    fn read_dialog_argb(hdlg: HWND) -> u32 {
        // SAFETY: hook의 dialog와 control ID가 유효하고 buffer가 충분하다.
        unsafe {
            let mut buf = [0u16; 32];

            let alpha = HOOK_CONTEXT.with(|ctx| {
                ctx.try_borrow()
                    .ok()
                    .and_then(|guard| guard.as_ref().map(|context| context.alpha as u32))
                    .unwrap_or(255)
            });

            // Red
            GetDlgItemTextW(
                hdlg,
                COLOR_RED_EDIT as i32,
                buf.as_mut_ptr(),
                buf.len() as i32,
            );
            let r_str = String::from_utf16_lossy(&buf);
            let r: u32 = r_str.trim_end_matches('\0').parse().unwrap_or(0);

            // Green
            GetDlgItemTextW(
                hdlg,
                COLOR_GREEN_EDIT as i32,
                buf.as_mut_ptr(),
                buf.len() as i32,
            );
            let g_str = String::from_utf16_lossy(&buf);
            let g: u32 = g_str.trim_end_matches('\0').parse().unwrap_or(0);

            // Blue
            GetDlgItemTextW(
                hdlg,
                COLOR_BLUE_EDIT as i32,
                buf.as_mut_ptr(),
                buf.len() as i32,
            );
            let b_str = String::from_utf16_lossy(&buf);
            let b: u32 = b_str.trim_end_matches('\0').parse().unwrap_or(0);

            ((alpha & 0xFF) << 24) | ((r & 0xFF) << 16) | ((g & 0xFF) << 8) | (b & 0xFF)
        }
    }

    fn notify_color_change(hdlg: HWND) {
        let color = Self::read_dialog_argb(hdlg);
        HOOK_CONTEXT.with(|ctx| {
            if let Ok(guard) = ctx.try_borrow()
                && let Some(ref context) = *guard
                && let Some(ref callback) = context.callback
            {
                callback(color);
            }
        });
    }

    /// `WM_INITDIALOG` 처리: no-activate 스타일 적용과 알파 컨트롤 생성.
    ///
    /// # Safety
    /// `hdlg`는 훅 프로시저가 받은 유효한 dialog handle이어야 한다.
    unsafe fn handle_init_dialog(hdlg: HWND) -> usize {
        // SAFETY: 호출자 계약상 hdlg는 유효한 dialog handle이다.
        unsafe {
            let (no_activate, show_alpha) = HOOK_CONTEXT.with(|ctx| {
                ctx.try_borrow()
                    .ok()
                    .and_then(|guard| {
                        guard
                            .as_ref()
                            .map(|context| (context.no_activate, context.show_alpha))
                    })
                    .unwrap_or((true, true))
            });

            if no_activate {
                let ex_style = GetWindowLongW(hdlg, GWL_EXSTYLE);
                SetWindowLongW(hdlg, GWL_EXSTYLE, ex_style | WS_EX_NOACTIVATE as i32);
            }

            if !show_alpha {
                return 1; // TRUE
            }

            // 다이얼로그 크기 확장 (알파 컨트롤 공간)
            let mut rect: RECT = zeroed();
            let _ = GetWindowRect(hdlg, &mut rect);
            let extra_width = Self::scale_for_dpi(60, hdlg);
            let _ = SetWindowPos(
                hdlg,
                std::ptr::null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left + extra_width,
                rect.bottom - rect.top,
                SWP_NOZORDER,
            );

            let hfont = GetStockObject(DEFAULT_GUI_FONT);
            let hinst = GetModuleHandleW(std::ptr::null());

            // 알파 트랙바 생성
            let trackbar = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                crate::win32::to_wide("msctls_trackbar32").as_ptr(),
                crate::win32::to_wide("").as_ptr(),
                TBS_VERT | TBS_BOTH | TBS_NOTICKS | WS_CHILD | WS_VISIBLE,
                Self::scale_for_dpi(540, hdlg),
                Self::scale_for_dpi(2, hdlg),
                Self::scale_for_dpi(25, hdlg),
                Self::scale_for_dpi(225, hdlg),
                hdlg,
                IDC_ALPHA_TRACKBAR as usize as *mut _,
                hinst,
                std::ptr::null(),
            );

            // Trackbar와 edit 중심에 맞춘 label.
            let label = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                crate::win32::to_wide("STATIC").as_ptr(),
                crate::win32::to_wide("불투명도").as_ptr(),
                // SS_CENTER = 0x1. (windows 0.62 에 상수 노출 안 됨)
                WS_CHILD | WS_VISIBLE | 0x1,
                Self::scale_for_dpi(522, hdlg),
                Self::scale_for_dpi(256, hdlg),
                Self::scale_for_dpi(60, hdlg),
                Self::scale_for_dpi(15, hdlg),
                hdlg,
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );

            // 에디트 박스 생성
            let edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                crate::win32::to_wide("EDIT").as_ptr(),
                crate::win32::to_wide("").as_ptr(),
                ES_CENTER as u32 | ES_AUTOHSCROLL as u32 | WS_CHILD | WS_VISIBLE | WS_BORDER,
                Self::scale_for_dpi(540, hdlg),
                Self::scale_for_dpi(230, hdlg),
                Self::scale_for_dpi(25, hdlg),
                Self::scale_for_dpi(18, hdlg),
                hdlg,
                IDC_ALPHA_EDIT as usize as *mut _,
                hinst,
                std::ptr::null(),
            );

            // lCustData에서 초기 알파값 가져오기
            let initial_alpha = HOOK_CONTEXT.with(|ctx| {
                ctx.try_borrow()
                    .ok()
                    .and_then(|g| g.as_ref().map(|c| c.alpha))
                    .unwrap_or(255)
            });

            // 트랙바 범위 설정 (0-255)
            let _ = SendDlgItemMessageW(
                hdlg,
                IDC_ALPHA_TRACKBAR as i32,
                TBM_SETRANGE,
                1,
                (255 << 16) as isize,
            );

            // 트랙바 초기 위치
            let _ = SendDlgItemMessageW(
                hdlg,
                IDC_ALPHA_TRACKBAR as i32,
                TBM_SETPOS,
                1,
                initial_alpha as isize,
            );

            // 에디트 초기값
            let alpha = crate::win32::to_wide(&initial_alpha.to_string());
            let _ = SetDlgItemTextW(hdlg, IDC_ALPHA_EDIT as i32, alpha.as_ptr());

            // 폰트 적용
            if !trackbar.is_null() {
                let _ = SendMessageW(trackbar, WM_SETFONT, hfont as usize, 0);
            }
            if !label.is_null() {
                let _ = SendMessageW(label, WM_SETFONT, hfont as usize, 0);
            }
            if !edit.is_null() {
                let _ = SendMessageW(edit, WM_SETFONT, hfont as usize, 0);
            }

            1 // TRUE
        }
    }

    /// `WM_HSCROLL`/`WM_VSCROLL` 처리: 알파 트랙바 이동을 에디트/콜백에 반영.
    ///
    /// # Safety
    /// `hdlg`는 훅 프로시저가 받은 유효한 dialog handle이어야 한다.
    unsafe fn handle_alpha_scroll(hdlg: HWND, wparam: WPARAM) -> usize {
        // SAFETY: 호출자 계약상 hdlg는 유효한 dialog handle이다.
        unsafe {
            // 트랙바 스크롤 처리
            let code = (wparam & 0xFFFF) as u32;
            let alpha = match super::trackbar_thumb_position(code, wparam) {
                Some(alpha) => alpha,
                None => match code {
                    TB_LINEUP | TB_LINEDOWN | TB_PAGEUP | TB_PAGEDOWN | TB_TOP | TB_BOTTOM
                    | TB_ENDTRACK => {
                        SendDlgItemMessageW(hdlg, IDC_ALPHA_TRACKBAR as i32, TBM_GETPOS, 0, 0)
                            as i32
                    }
                    _ => return 0,
                },
            };

            // 에디트 업데이트
            let alpha_text = crate::win32::to_wide(&alpha.to_string());
            let _ = SetDlgItemTextW(hdlg, IDC_ALPHA_EDIT as i32, alpha_text.as_ptr());

            // 컨텍스트에 알파값 저장
            HOOK_CONTEXT.with(|ctx| {
                if let Ok(mut guard) = ctx.try_borrow_mut()
                    && let Some(ref mut c) = *guard
                {
                    c.alpha = alpha;
                }
            });

            Self::notify_color_change(hdlg);
            0 // FALSE
        }
    }

    /// `WM_COMMAND` 처리: 알파/RGB 에디트 변경을 콜백에 반영.
    ///
    /// # Safety
    /// `hdlg`는 훅 프로시저가 받은 유효한 dialog handle이어야 한다.
    unsafe fn handle_command(hdlg: HWND, wparam: WPARAM) -> usize {
        // SAFETY: 호출자 계약상 hdlg는 유효한 dialog handle이다.
        unsafe {
            let id = (wparam & 0xFFFF) as u16;
            let notification = ((wparam >> 16) & 0xFFFF) as u32;
            if id == IDC_ALPHA_EDIT && matches!(notification, EN_CHANGE | EN_KILLFOCUS) {
                let mut buffer = [0u16; 16];
                GetDlgItemTextW(
                    hdlg,
                    IDC_ALPHA_EDIT as i32,
                    buffer.as_mut_ptr(),
                    buffer.len() as i32,
                );
                let text = String::from_utf16_lossy(&buffer);
                if let Ok(value) = text.trim_end_matches('\0').parse::<i32>() {
                    let alpha = value.clamp(0, 255);
                    let _ = SendDlgItemMessageW(
                        hdlg,
                        IDC_ALPHA_TRACKBAR as i32,
                        TBM_SETPOS,
                        1,
                        alpha as isize,
                    );
                    HOOK_CONTEXT.with(|ctx| {
                        if let Ok(mut guard) = ctx.try_borrow_mut()
                            && let Some(ref mut context) = *guard
                        {
                            context.alpha = alpha;
                        }
                    });
                    if notification == EN_KILLFOCUS && value != alpha {
                        let value = crate::win32::to_wide(&alpha.to_string());
                        let _ = SetDlgItemTextW(hdlg, IDC_ALPHA_EDIT as i32, value.as_ptr());
                    }
                    Self::notify_color_change(hdlg);
                }
            } else if matches!(id, COLOR_RED_EDIT | COLOR_GREEN_EDIT | COLOR_BLUE_EDIT)
                && notification == EN_CHANGE
            {
                // 팔레트, 색상 스펙트럼, 키보드/직접 입력 모두 RGB edit을 갱신한다.
                Self::notify_color_change(hdlg);
            }
            0 // FALSE
        }
    }

    /// `WM_MOVING`/`WM_SIZING` 처리: `WS_EX_NOACTIVATE` 스타일 적용 시 위치/크기 변경 강제.
    ///
    /// # Safety
    /// `hdlg`는 훅 프로시저가 받은 유효한 dialog handle이고, `lparam`은 해당
    /// message의 `RECT` 포인터여야 한다.
    unsafe fn handle_moving_or_sizing(hdlg: HWND, lparam: LPARAM) -> usize {
        // SAFETY: 호출자 계약상 lparam은 WM_MOVING/WM_SIZING의 RECT 포인터다.
        unsafe {
            let prc = lparam as *mut RECT;
            if !prc.is_null() {
                let rc = &*prc;
                let _ = SetWindowPos(
                    hdlg,
                    std::ptr::null_mut(),
                    rc.left,
                    rc.top,
                    rc.right - rc.left,
                    rc.bottom - rc.top,
                    SWP_NOZORDER,
                );
            }
            0 // FALSE
        }
    }

    /// CHOOSECOLOR 훅 프로시저
    // SAFETY: Windows가 유효한 dialog와 message pointer로 같은 thread에서 호출한다.
    unsafe extern "system" fn hook_proc(
        hdlg: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> usize {
        unsafe {
            match msg {
                WM_INITDIALOG => Self::handle_init_dialog(hdlg),
                WM_HSCROLL | WM_VSCROLL => Self::handle_alpha_scroll(hdlg, wparam),
                WM_COMMAND => Self::handle_command(hdlg, wparam),
                WM_MOVING | WM_SIZING => Self::handle_moving_or_sizing(hdlg, lparam),
                _ => 0, // FALSE
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/dialogs/color.rs"]
mod tests;
