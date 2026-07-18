//! Alpha control과 실시간 callback을 추가한 `CHOOSECOLOR` dialog.

use std::cell::RefCell;
use std::mem::zeroed;

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
        UI::Controls::Dialogs::*, UI::Controls::*, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::constants::{COLOR_BLUE_EDIT, COLOR_GREEN_EDIT, COLOR_RED_EDIT, TBM_GETPOS_VAL};

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
}

impl Default for ColorDialogConfig {
    fn default() -> Self {
        Self {
            initial_color: 0xFF000000, // 불투명 검정
            on_color_change: None,
            no_activate: true,
        }
    }
}

/// 훅 프로시저용 컨텍스트
struct HookContext {
    alpha: i32,
    callback: Option<ColorChangeCallback>,
    no_activate: bool,
}

thread_local! {
    static HOOK_CONTEXT: RefCell<Option<HookContext>> = const { RefCell::new(None) };
    static CUSTOM_COLORS: RefCell<[COLORREF; 16]> = const { RefCell::new([COLORREF(0xFFFFFF); 16]) };
}

pub struct ColorDialog;

impl ColorDialog {
    /// 색상 선택 대화상자 표시
    pub fn show(hwnd: HWND, config: ColorDialogConfig) -> Option<ColorResult> {
        // SAFETY: 호출자가 유효한 hwnd를 제공한다.
        unsafe { Self::show_impl(hwnd, config) }
    }

    /// 간단한 색상 선택 (콜백 없음)
    pub fn show_simple(hwnd: HWND, initial_argb: u32) -> Option<ColorResult> {
        Self::show(
            hwnd,
            ColorDialogConfig {
                initial_color: initial_argb,
                on_color_change: None,
                no_activate: false,
            },
        )
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
                    });
                }
            });

            let mut custom_colors = CUSTOM_COLORS.with(|c| {
                c.try_borrow()
                    .map(|g| *g)
                    .unwrap_or([COLORREF(0xFFFFFF); 16])
            });

            let mut cc: CHOOSECOLORW = zeroed();
            cc.lStructSize = std::mem::size_of::<CHOOSECOLORW>() as u32;
            cc.hwndOwner = hwnd;
            cc.lpCustColors = custom_colors.as_mut_ptr();
            cc.rgbResult = COLORREF(initial_colorref);
            cc.lCustData = LPARAM(alpha as isize);
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

            if result.as_bool() {
                Some(ColorResult::from_colorref(cc.rgbResult.0, final_alpha))
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

            // 알파
            GetDlgItemTextW(hdlg, IDC_ALPHA_EDIT as i32, &mut buf);
            let alpha_str = String::from_utf16_lossy(&buf);
            let alpha: u32 = alpha_str
                .trim_end_matches('\0')
                .parse::<u32>()
                .unwrap_or(255)
                .min(255);

            // Red
            GetDlgItemTextW(hdlg, COLOR_RED_EDIT as i32, &mut buf);
            let r_str = String::from_utf16_lossy(&buf);
            let r: u32 = r_str.trim_end_matches('\0').parse().unwrap_or(0);

            // Green
            GetDlgItemTextW(hdlg, COLOR_GREEN_EDIT as i32, &mut buf);
            let g_str = String::from_utf16_lossy(&buf);
            let g: u32 = g_str.trim_end_matches('\0').parse().unwrap_or(0);

            // Blue
            GetDlgItemTextW(hdlg, COLOR_BLUE_EDIT as i32, &mut buf);
            let b_str = String::from_utf16_lossy(&buf);
            let b: u32 = b_str.trim_end_matches('\0').parse().unwrap_or(0);

            ((alpha & 0xFF) << 24) | ((r & 0xFF) << 16) | ((g & 0xFF) << 8) | (b & 0xFF)
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
                WM_INITDIALOG => {
                    // 다이얼로그 크기 확장 (알파 컨트롤 공간)
                    let mut rect: RECT = zeroed();
                    let _ = GetWindowRect(hdlg, &mut rect);
                    let extra_width = Self::scale_for_dpi(60, hdlg);
                    let _ = SetWindowPos(
                        hdlg,
                        None,
                        rect.left,
                        rect.top,
                        rect.right - rect.left + extra_width,
                        rect.bottom - rect.top,
                        SWP_NOZORDER,
                    );

                    let hfont = GetStockObject(DEFAULT_GUI_FONT);
                    let hinst = GetModuleHandleW(None).unwrap_or_default();

                    // 알파 트랙바 생성
                    let trackbar = CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        w!("msctls_trackbar32"),
                        w!(""),
                        WINDOW_STYLE(TBS_VERT | TBS_BOTH | TBS_NOTICKS | WS_CHILD.0 | WS_VISIBLE.0),
                        Self::scale_for_dpi(540, hdlg),
                        Self::scale_for_dpi(2, hdlg),
                        Self::scale_for_dpi(25, hdlg),
                        Self::scale_for_dpi(225, hdlg),
                        Some(hdlg),
                        Some(HMENU(IDC_ALPHA_TRACKBAR as isize as *mut _)),
                        Some(hinst.into()),
                        None,
                    );

                    // Trackbar와 edit 중심에 맞춘 label.
                    let label = CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        w!("STATIC"),
                        w!("불투명도"),
                        // SS_CENTER = 0x1. (windows 0.62 에 상수 노출 안 됨)
                        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0x1),
                        Self::scale_for_dpi(522, hdlg),
                        Self::scale_for_dpi(256, hdlg),
                        Self::scale_for_dpi(60, hdlg),
                        Self::scale_for_dpi(15, hdlg),
                        Some(hdlg),
                        None,
                        Some(hinst.into()),
                        None,
                    );

                    // 에디트 박스 생성
                    let edit = CreateWindowExW(
                        WS_EX_CLIENTEDGE,
                        w!("EDIT"),
                        w!(""),
                        WINDOW_STYLE(
                            ES_CENTER as u32
                                | ES_AUTOHSCROLL as u32
                                | WS_CHILD.0
                                | WS_VISIBLE.0
                                | WS_BORDER.0,
                        ),
                        Self::scale_for_dpi(540, hdlg),
                        Self::scale_for_dpi(230, hdlg),
                        Self::scale_for_dpi(25, hdlg),
                        Self::scale_for_dpi(18, hdlg),
                        Some(hdlg),
                        Some(HMENU(IDC_ALPHA_EDIT as isize as *mut _)),
                        Some(hinst.into()),
                        None,
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
                        WPARAM(1),
                        LPARAM((255 << 16) as isize),
                    );

                    // 트랙바 초기 위치
                    let _ = SendDlgItemMessageW(
                        hdlg,
                        IDC_ALPHA_TRACKBAR as i32,
                        TBM_SETPOS,
                        WPARAM(1),
                        LPARAM(initial_alpha as isize),
                    );

                    // 에디트 초기값
                    let alpha = HSTRING::from(initial_alpha.to_string());
                    if let Err(e) = SetDlgItemTextW(hdlg, IDC_ALPHA_EDIT as i32, &alpha) {
                        tracing::warn!("SetDlgItemTextW failed: {e}");
                    }

                    // 폰트 적용
                    if let Ok(trackbar) = trackbar {
                        let _ = SendMessageW(
                            trackbar,
                            WM_SETFONT,
                            Some(WPARAM(hfont.0 as usize)),
                            Some(LPARAM(0)),
                        );
                    }
                    if let Ok(label) = label {
                        let _ = SendMessageW(
                            label,
                            WM_SETFONT,
                            Some(WPARAM(hfont.0 as usize)),
                            Some(LPARAM(0)),
                        );
                    }
                    if let Ok(edit) = edit {
                        let _ = SendMessageW(
                            edit,
                            WM_SETFONT,
                            Some(WPARAM(hfont.0 as usize)),
                            Some(LPARAM(0)),
                        );
                    }

                    // WS_EX_NOACTIVATE 설정
                    let no_activate = HOOK_CONTEXT.with(|ctx| {
                        ctx.try_borrow()
                            .ok()
                            .and_then(|g| g.as_ref().map(|c| c.no_activate))
                            .unwrap_or(true)
                    });

                    if no_activate {
                        let ex_style = GetWindowLongW(hdlg, GWL_EXSTYLE);
                        SetWindowLongW(hdlg, GWL_EXSTYLE, ex_style | WS_EX_NOACTIVATE.0 as i32);
                    }

                    return 1; // TRUE
                }

                WM_HSCROLL | WM_VSCROLL => {
                    // 트랙바 스크롤 처리
                    let code = (wparam.0 & 0xFFFF) as u32;
                    let alpha = match code {
                        TB_THUMBTRACK => ((wparam.0 >> 16) & 0xFFFF) as i32,
                        TB_LINEUP | TB_LINEDOWN | TB_PAGEUP | TB_PAGEDOWN | TB_TOP | TB_BOTTOM
                        | TB_ENDTRACK => {
                            SendDlgItemMessageW(
                                hdlg,
                                IDC_ALPHA_TRACKBAR as i32,
                                TBM_GETPOS_VAL,
                                WPARAM(0),
                                LPARAM(0),
                            )
                            .0 as i32
                        }
                        _ => return 0,
                    };

                    // 에디트 업데이트
                    let alpha_text = HSTRING::from(alpha.to_string());
                    let _ = SetDlgItemTextW(hdlg, IDC_ALPHA_EDIT as i32, &alpha_text);

                    // 컨텍스트에 알파값 저장
                    HOOK_CONTEXT.with(|ctx| {
                        if let Ok(mut guard) = ctx.try_borrow_mut()
                            && let Some(ref mut c) = *guard
                        {
                            c.alpha = alpha;
                        }
                    });

                    // 콜백 호출
                    let color = Self::read_dialog_argb(hdlg);
                    HOOK_CONTEXT.with(|ctx| {
                        if let Ok(guard) = ctx.try_borrow()
                            && let Some(ref c) = *guard
                            && let Some(ref cb) = c.callback
                        {
                            cb(color);
                        }
                    });
                }

                WM_COMMAND => {
                    let id = (wparam.0 & 0xFFFF) as u16;
                    let notification = ((wparam.0 >> 16) & 0xFFFF) as u32;
                    if id == IDC_ALPHA_EDIT && matches!(notification, EN_CHANGE | EN_KILLFOCUS) {
                        let mut buffer = [0u16; 16];
                        GetDlgItemTextW(hdlg, IDC_ALPHA_EDIT as i32, &mut buffer);
                        let text = String::from_utf16_lossy(&buffer);
                        if let Ok(value) = text.trim_end_matches('\0').parse::<i32>() {
                            let alpha = value.clamp(0, 255);
                            let _ = SendDlgItemMessageW(
                                hdlg,
                                IDC_ALPHA_TRACKBAR as i32,
                                TBM_SETPOS,
                                WPARAM(1),
                                LPARAM(alpha as isize),
                            );
                            HOOK_CONTEXT.with(|ctx| {
                                if let Ok(mut guard) = ctx.try_borrow_mut()
                                    && let Some(ref mut context) = *guard
                                {
                                    context.alpha = alpha;
                                }
                            });
                            if notification == EN_KILLFOCUS && value != alpha {
                                let value = HSTRING::from(alpha.to_string());
                                let _ = SetDlgItemTextW(hdlg, IDC_ALPHA_EDIT as i32, &value);
                            }
                        }
                    }
                }

                WM_KEYDOWN | WM_KEYUP | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN
                | WM_RBUTTONUP | WM_MOUSEMOVE => {
                    // 색상 변경 시 콜백 호출
                    let color = Self::read_dialog_argb(hdlg);
                    HOOK_CONTEXT.with(|ctx| {
                        if let Ok(guard) = ctx.try_borrow()
                            && let Some(ref c) = *guard
                            && let Some(ref cb) = c.callback
                        {
                            cb(color);
                        }
                    });
                }

                WM_MOVING | WM_SIZING => {
                    // WS_EX_NOACTIVATE 스타일 적용 시 위치/크기 변경 강제
                    let prc = lparam.0 as *mut RECT;
                    if !prc.is_null() {
                        let rc = &*prc;
                        let _ = SetWindowPos(
                            hdlg,
                            None,
                            rc.left,
                            rc.top,
                            rc.right - rc.left,
                            rc.bottom - rc.top,
                            SWP_NOZORDER,
                        );
                    }
                }

                _ => {}
            }

            0 // FALSE
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/dialogs/color.rs"]
mod tests;
