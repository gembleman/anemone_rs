//! Per-Monitor V2 control 좌표와 font scaling helper.

use windows::Win32::{
    Foundation::HWND,
    UI::HiDpi::{GetDpiForSystem, GetDpiForWindow},
    UI::WindowsAndMessaging::USER_DEFAULT_SCREEN_DPI,
};

/// 디자인 기준 DPI (96 = 100%)
pub const BASE_DPI: u32 = USER_DEFAULT_SCREEN_DPI;

/// 창 DPI를 반환하며 실패하면 system DPI, 이어서 `BASE_DPI`로 fallback한다.
pub fn dpi_for_window(hwnd: HWND) -> u32 {
    // SAFETY: GetDpiForWindow / GetDpiForSystem 모두 부수효과 없는 user32
    // 함수이며, null HWND 에는 0 을 반환하므로 결과만 검증하면 안전하다.
    unsafe {
        if !hwnd.0.is_null() {
            let dpi = GetDpiForWindow(hwnd);
            if dpi > 0 {
                return dpi;
            }
        }
        let sys = GetDpiForSystem();
        if sys > 0 { sys } else { BASE_DPI }
    }
}

/// 96 DPI 기준 좌표/크기를 윈도우의 실제 DPI 로 스케일링한다.
#[inline]
pub fn scale(value: i32, dpi: u32) -> i32 {
    // MulDiv 대신 i64 계산: i32 범위에서 안전하며 같은 결과.
    ((value as i64) * (dpi as i64) / (BASE_DPI as i64)) as i32
}

/// 96 DPI 기준 좌표/크기를 윈도우의 DPI 로 스케일링한다.
#[inline]
pub fn scale_for_window(value: i32, hwnd: HWND) -> i32 {
    scale(value, dpi_for_window(hwnd))
}
