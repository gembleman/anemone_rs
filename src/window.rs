use std::mem::zeroed;

use windows::Win32::{Foundation::*, Graphics::Gdi::ScreenToClient, UI::WindowsAndMessaging::*};

/// 텍스트 렌더링 스타일
#[derive(Clone, Debug)]
pub struct TextRenderStyle {
    pub font_size: i32,
    pub font_face: String,
    pub font_style: u8, // 0: normal, 1: bold, 2: italic, 3: bold+italic
    pub color: u32,     // ARGB
    pub outline1_size: i32,
    pub outline1_color: u32,
    pub outline2_size: i32,
    pub outline2_color: u32,
    pub shadow_enabled: bool,
    pub shadow_color: u32,
    pub shadow_offset_x: i32,
    pub shadow_offset_y: i32,
}

/// 윈도우 표시/숨김
pub fn set_window_visible(hwnd: HWND, visible: bool) {
    // SAFETY: hwnd is a valid window handle from the caller.
    unsafe {
        let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
    }
}

/// 최상위 설정
pub fn set_topmost(hwnd: HWND, topmost: bool) {
    // SAFETY: hwnd is a valid window handle from the caller.
    unsafe {
        let hwnd_insert = if topmost {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        };
        let _ = SetWindowPos(
            hwnd,
            Some(hwnd_insert),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// 클릭 통과 설정
pub fn set_click_through(hwnd: HWND, click_through: bool) {
    // SAFETY: hwnd is a valid window handle. GetWindowLongW/SetWindowLongW modify the
    // extended window style which is a safe operation on a valid window.
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let new_style = if click_through {
            style | WS_EX_TRANSPARENT.0 as i32
        } else {
            style & !(WS_EX_TRANSPARENT.0 as i32)
        };
        SetWindowLongW(hwnd, GWL_EXSTYLE, new_style);
    }
}

/// WM_NCHITTEST 처리 - 테두리 크기 조절 영역 판정
pub fn hit_test_resize_border(hwnd: HWND, x: i32, y: i32, border_width: i32) -> Option<i32> {
    // SAFETY: hwnd is a valid window handle. GetClientRect and ScreenToClient are standard
    // Win32 coordinate conversion calls with valid parameters.
    unsafe {
        let mut rc: RECT = zeroed();
        GetClientRect(hwnd, &mut rc).ok()?;

        let mut pt = POINT { x, y };
        let _ = ScreenToClient(hwnd, &mut pt);

        let w = rc.right;
        let h = rc.bottom;

        // 상단
        if pt.y < border_width {
            if pt.x < border_width {
                return Some(HTTOPLEFT as i32);
            } else if pt.x > w - border_width {
                return Some(HTTOPRIGHT as i32);
            }
            return Some(HTTOP as i32);
        }

        // 하단
        if pt.y > h - border_width {
            if pt.x < border_width {
                return Some(HTBOTTOMLEFT as i32);
            } else if pt.x > w - border_width {
                return Some(HTBOTTOMRIGHT as i32);
            }
            return Some(HTBOTTOM as i32);
        }

        // 좌우
        if pt.x < border_width {
            return Some(HTLEFT as i32);
        }
        if pt.x > w - border_width {
            return Some(HTRIGHT as i32);
        }

        None // 클라이언트 영역
    }
}

/// 윈도우 최소 크기 설정
pub fn set_min_track_size(mm: &mut MINMAXINFO, min_width: i32, min_height: i32) {
    mm.ptMinTrackSize.x = min_width;
    mm.ptMinTrackSize.y = min_height;
}
