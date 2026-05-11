//! DPI 스케일링 헬퍼
//!
//! Windows 10 1607+ 가 제공하는 per-window DPI API (`GetDpiForWindow`) 와
//! 1607+ `GetDpiForSystem` 을 직접 사용한다. 매니페스트에서 Per-Monitor V2 를
//! 선언하므로 자식 컨트롤 좌표/폰트를 명시적으로 스케일링한다.

use windows::Win32::{
    Foundation::HWND,
    UI::HiDpi::{GetDpiForSystem, GetDpiForWindow},
    UI::WindowsAndMessaging::USER_DEFAULT_SCREEN_DPI,
};

/// 디자인 기준 DPI (96 = 100%)
pub const BASE_DPI: u32 = USER_DEFAULT_SCREEN_DPI;

/// 지정한 윈도우의 DPI 를 반환한다.
///
/// `hwnd` 가 null 이거나 호출이 실패하면 시스템 DPI 로, 그것도 0 이면
/// `BASE_DPI` 로 폴백한다.
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

/// 시스템 DPI 기준으로 폰트 크기를 스케일링 (윈도우 핸들이 없을 때).
#[inline]
pub fn scale_font_for_system(logical_height: i32) -> i32 {
    // SAFETY: GetDpiForSystem 은 부수효과 없는 user32 함수.
    let dpi = unsafe { GetDpiForSystem() };
    let dpi = if dpi > 0 { dpi } else { BASE_DPI };
    scale(logical_height, dpi)
}
