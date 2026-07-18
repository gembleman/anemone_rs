//! 폰트 선택 대화상자
//!
//! CHOOSEFONT 다이얼로그 래퍼.

use std::mem::zeroed;

use windows::Win32::{
    Foundation::*, Graphics::Gdi::*, UI::Controls::Dialogs::*, UI::WindowsAndMessaging::*,
};

/// 폰트 스타일 플래그
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FontStyle {
    pub bold: bool,
    pub italic: bool,
}

impl FontStyle {
    pub fn from_bits(bits: u8) -> Self {
        Self {
            bold: bits & 1 != 0,
            italic: bits & 2 != 0,
        }
    }

    pub fn to_bits(self) -> u8 {
        let mut bits = 0u8;
        if self.bold {
            bits |= 1;
        }
        if self.italic {
            bits |= 2;
        }
        bits
    }
}

/// 폰트 선택 결과
#[derive(Debug, Clone)]
pub struct FontResult {
    /// 폰트 패밀리 이름
    pub face_name: String,
    /// 폰트 스타일 (bold, italic)
    pub style: FontStyle,
    /// 선택된 포인트 크기 (pt)
    pub point_size: i32,
}

impl FontResult {
    /// CHOOSEFONT 결과에서 변환.
    /// `cf.iPointSize` 는 1/10 pt 단위로 채워져 돌아온다.
    fn from_choosefont(lf: &LOGFONTW, cf: &CHOOSEFONTW) -> Self {
        let face_name = String::from_utf16_lossy(
            &lf.lfFaceName[..lf
                .lfFaceName
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(lf.lfFaceName.len())],
        );

        Self {
            face_name,
            style: FontStyle {
                bold: lf.lfWeight >= 700,
                italic: lf.lfItalic != 0,
            },
            point_size: (cf.iPointSize + 5) / 10,
        }
    }
}

/// 폰트 대화상자 설정
#[derive(Default)]
pub struct FontDialogConfig {
    /// 초기 폰트 이름
    pub initial_face: Option<String>,
    /// 초기 스타일
    pub initial_style: FontStyle,
    /// 초기 포인트 크기
    pub initial_point_size: i32,
    /// WS_EX_NOACTIVATE 스타일 적용
    pub no_activate: bool,
}

/// 폰트 대화상자
pub struct FontDialog;

impl FontDialog {
    /// 폰트 선택 대화상자 표시
    pub fn show(hwnd: HWND, config: FontDialogConfig) -> Option<FontResult> {
        // SAFETY: hwnd is a valid window handle from the caller. show_impl handles all
        // Win32 dialog setup with valid parameters.
        unsafe { Self::show_impl(hwnd, config) }
    }

    unsafe fn show_impl(hwnd: HWND, config: FontDialogConfig) -> Option<FontResult> {
        // SAFETY: hwnd is a valid window handle. CHOOSEFONTW is initialized with correct
        // lStructSize, valid owner handle, and valid lpLogFont pointer to stack-allocated
        // LOGFONTW. zeroed() produces valid default state. The hook procedure pointer
        // (if set) is a valid extern "system" fn.
        unsafe {
            let mut lf: LOGFONTW = zeroed();

            // 초기 폰트 이름 설정
            if let Some(ref face) = config.initial_face {
                let face_utf16: Vec<u16> = face.encode_utf16().collect();
                let copy_len = face_utf16.len().min(lf.lfFaceName.len() - 1);
                lf.lfFaceName[..copy_len].copy_from_slice(&face_utf16[..copy_len]);
            }

            // 초기 스타일
            lf.lfWeight = if config.initial_style.bold { 700 } else { 400 };
            lf.lfItalic = if config.initial_style.italic { 1 } else { 0 };

            // 포인트 크기를 픽셀로 변환 (Per-Monitor V2 정확도)
            if config.initial_point_size > 0 {
                let dpi = crate::dpi::dpi_for_window(hwnd) as i32;
                lf.lfHeight = -((config.initial_point_size * dpi) / 72);
            } else {
                lf.lfHeight = -16; // 기본 12pt 정도
            }

            let mut cf: CHOOSEFONTW = zeroed();
            cf.lStructSize = std::mem::size_of::<CHOOSEFONTW>() as u32;
            cf.hwndOwner = hwnd;
            cf.lpLogFont = &mut lf;
            cf.iPointSize = config.initial_point_size.max(10) * 10; // 1/10 pt 단위

            // 플래그: 화면 폰트만, 세로쓰기 제외, 스크립트 선택 제외
            cf.Flags = CF_SCREENFONTS | CF_NOVERTFONTS | CF_INITTOLOGFONTSTRUCT | CF_NOSCRIPTSEL;

            if config.no_activate {
                let mut flags = cf.Flags;
                flags |= CF_ENABLEHOOK;
                cf.Flags = flags;
                cf.lpfnHook = Some(Self::hook_proc_noactivate);
            }

            cf.rgbColors = COLORREF(0);
            cf.nFontType = SCREEN_FONTTYPE;

            if ChooseFontW(&mut cf).as_bool() {
                Some(FontResult::from_choosefont(&lf, &cf))
            } else {
                None
            }
        }
    }

    /// WS_EX_NOACTIVATE 훅 프로시저
    // SAFETY: This is a CHOOSEFONT hook procedure called by the system. hdlg is a valid
    // dialog handle provided by Windows. GetWindowLongW/SetWindowLongW use the valid hdlg.
    // The RECT pointer from lparam in WM_MOVING/WM_SIZING is valid per the Win32 contract.
    // Null check is performed before dereferencing.
    unsafe extern "system" fn hook_proc_noactivate(
        hdlg: HWND,
        msg: u32,
        _wparam: WPARAM,
        lparam: LPARAM,
    ) -> usize {
        unsafe {
            match msg {
                WM_INITDIALOG => {
                    // WS_EX_NOACTIVATE 스타일 추가
                    let ex_style = GetWindowLongW(hdlg, GWL_EXSTYLE);
                    SetWindowLongW(hdlg, GWL_EXSTYLE, ex_style | WS_EX_NOACTIVATE.0 as i32);
                    return 1; // TRUE
                }

                WM_MOVING | WM_SIZING => {
                    // WS_EX_NOACTIVATE 상태에서 위치/크기 변경 강제
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
#[path = "../../tests/unit/dialogs/font.rs"]
mod tests;
