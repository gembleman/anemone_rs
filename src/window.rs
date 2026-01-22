use std::mem::zeroed;
use std::ptr::null_mut;

use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::WindowsAndMessaging::*,
    },
};

/// 더블 버퍼링용 DIB 섹션
pub struct DoubleBuffer {
    hdc_mem: HDC,
    hbitmap: HBITMAP,
    hbitmap_old: HGDIOBJ,
    bits: *mut u8,
    pub width: i32,
    pub height: i32,
}

impl Drop for DoubleBuffer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.hdc_mem, self.hbitmap_old);
            let _ = DeleteObject(self.hbitmap.into());
            let _ = DeleteDC(self.hdc_mem);
        }
    }
}

impl DoubleBuffer {
    pub fn new(hdc: HDC, width: i32, height: i32) -> Result<Self> {
        unsafe {
            let hdc_mem = CreateCompatibleDC(Some(hdc));
            if hdc_mem.is_invalid() {
                return Err(Error::from_hresult(HRESULT::from_win32(GetLastError().0)));
            }

            let mut bmi: BITMAPINFO = zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = width;
            bmi.bmiHeader.biHeight = -height; // top-down DIB
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;

            let mut bits: *mut std::ffi::c_void = null_mut();
            let hbitmap = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;

            let hbitmap_old = SelectObject(hdc_mem, hbitmap.into());

            Ok(Self {
                hdc_mem,
                hbitmap,
                hbitmap_old,
                bits: bits as *mut u8,
                width,
                height,
            })
        }
    }

    pub fn clear(&mut self, r: u8, g: u8, b: u8, a: u8) {
        unsafe {
            let pixel_count = (self.width * self.height) as usize;
            let pixels = std::slice::from_raw_parts_mut(self.bits as *mut u32, pixel_count);
            // BGRA 순서 (DIB는 BGRA)
            let color = ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            pixels.fill(color);
        }
    }

    pub fn hdc(&self) -> HDC {
        self.hdc_mem
    }

    /// 사각형 채우기 (ARGB)
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        let a = ((color >> 24) & 0xFF) as u8;
        let r = ((color >> 16) & 0xFF) as u8;
        let g = ((color >> 8) & 0xFF) as u8;
        let b = (color & 0xFF) as u8;
        let bgra = ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);

        let width = self.width;
        let height = self.height;

        unsafe {
            let pixel_count = (width * height) as usize;
            let pixels = std::slice::from_raw_parts_mut(self.bits as *mut u32, pixel_count);

            for py in y.max(0)..(y + h).min(height) {
                for px in x.max(0)..(x + w).min(width) {
                    let idx = (py * width + px) as usize;
                    if idx < pixels.len() {
                        pixels[idx] = bgra;
                    }
                }
            }
        }
    }

    /// 테두리 그리기 (ARGB)
    pub fn draw_border(&mut self, thickness: i32, color: u32) {
        let w = self.width;
        let h = self.height;

        // 상단
        self.fill_rect(0, 0, w, thickness, color);
        // 하단
        self.fill_rect(0, h - thickness, w, thickness, color);
        // 좌측
        self.fill_rect(0, 0, thickness, h, color);
        // 우측
        self.fill_rect(w - thickness, 0, thickness, h, color);
    }
}

/// 레이어드 윈도우 업데이트
pub fn update_layered_window(hwnd: HWND, buffer: &DoubleBuffer) -> Result<()> {
    unsafe {
        let hdc_screen = GetDC(None);
        let size = SIZE {
            cx: buffer.width,
            cy: buffer.height,
        };
        let pt_src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        UpdateLayeredWindow(
            hwnd,
            Some(hdc_screen),
            None,
            Some(&size),
            Some(buffer.hdc()),
            Some(&pt_src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )?;

        ReleaseDC(None, hdc_screen);
        Ok(())
    }
}

/// 윈도우 표시/숨김
pub fn set_window_visible(hwnd: HWND, visible: bool) {
    unsafe {
        let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
    }
}

/// 최상위 설정
#[allow(dead_code)]
pub fn set_topmost(hwnd: HWND, topmost: bool) {
    unsafe {
        let hwnd_insert = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
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
