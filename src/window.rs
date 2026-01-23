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

/// 텍스트 렌더링 스타일
#[derive(Clone, Debug)]
pub struct TextRenderStyle {
    pub font_size: i32,
    pub font_face: String,
    pub font_style: u8,  // 0: normal, 1: bold, 2: italic, 3: bold+italic
    pub color: u32,      // ARGB
    pub outline1_size: i32,
    pub outline1_color: u32,
    pub outline2_size: i32,
    pub outline2_color: u32,
    pub shadow_enabled: bool,
    pub shadow_color: u32,
    pub shadow_offset_x: i32,
    pub shadow_offset_y: i32,
}

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

    /// 텍스트 그리기 (GDI 사용) - 외곽선, 그림자 지원
    /// 원본 아네모네의 GDI+ 로직을 GDI로 시뮬레이션:
    /// - 외곽선2(OutlineOut): outline1 + outline2 두께
    /// - 외곽선1(OutlineIn): outline1 두께
    /// - 그림자: 오프셋 위치에 외곽선 총 두께로 그림
    pub fn draw_text(&mut self, text: &str, x: i32, y: i32, style: &TextRenderStyle) {
        unsafe {
            // 폰트 스타일 파싱
            let weight = if style.font_style & 1 != 0 { FW_BOLD.0 as i32 } else { FW_NORMAL.0 as i32 };
            let italic = if style.font_style & 2 != 0 { 1u32 } else { 0u32 };

            // 폰트 이름을 wide string으로 변환
            let font_face_wide: Vec<u16> = style.font_face.encode_utf16().chain(std::iter::once(0)).collect();

            // 폰트 생성
            let font = CreateFontW(
                style.font_size,       // 높이
                0,                     // 너비 (0 = 자동)
                0,                     // escapement
                0,                     // orientation
                weight,                // weight
                italic,                // italic
                0,                     // underline
                0,                     // strikeout
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                DEFAULT_QUALITY,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                PCWSTR(font_face_wide.as_ptr()),
            );

            let old_font = SelectObject(self.hdc_mem, font.into());
            SetBkMode(self.hdc_mem, TRANSPARENT);

            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let text_slice = &wide[..wide.len()-1];

            // 원본 아네모네 로직: outlineTotalThick = outlineIn + outlineOut
            let outline_total = style.outline1_size + style.outline2_size;

            // 1. 그림자 그리기 (가장 뒤에)
            // 원본: 그림자는 오프셋 위치에 외곽선 총 두께로 DrawPath
            if style.shadow_enabled && (style.shadow_offset_x != 0 || style.shadow_offset_y != 0) {
                let shadow_x = x + style.shadow_offset_x;
                let shadow_y = y + style.shadow_offset_y;

                if outline_total > 0 {
                    // 그림자도 외곽선과 함께 그림
                    self.draw_outline_at(text_slice, shadow_x, shadow_y, outline_total, style.shadow_color);
                }
                // 그림자 텍스트 본체
                self.set_text_color(style.shadow_color);
                let _ = TextOutW(self.hdc_mem, shadow_x, shadow_y, text_slice);
            }

            // 2. 외곽선2 그리기 (OutlineOut) - outline1 + outline2 두께
            // 원본: outlineTotalThick 두께로 DrawPath
            if style.outline2_size > 0 && outline_total > 0 {
                self.draw_outline_at(text_slice, x, y, outline_total, style.outline2_color);
            }

            // 3. 외곽선1 그리기 (OutlineIn) - outline1 두께만
            // 원본: outlineInThick 두께로 DrawPath
            if style.outline1_size > 0 {
                self.draw_outline_at(text_slice, x, y, style.outline1_size, style.outline1_color);
            }

            // 4. 주 텍스트 그리기 (가장 앞에)
            // 원본: FillPath로 텍스트 내부 채움
            self.set_text_color(style.color);
            let _ = TextOutW(self.hdc_mem, x, y, text_slice);

            // 정리
            SelectObject(self.hdc_mem, old_font);
            let _ = DeleteObject(font.into());

            // premultiplied alpha 적용 (레이어드 윈도우용)
            self.apply_text_alpha_full();
        }
    }

    /// 지정된 위치에 외곽선 그리기 (8방향 + 추가 픽셀)
    fn draw_outline_at(&self, text: &[u16], x: i32, y: i32, thickness: i32, color: u32) {
        self.set_text_color(color);

        // 외곽선 두께만큼 모든 방향으로 텍스트 그리기
        for dy in -thickness..=thickness {
            for dx in -thickness..=thickness {
                // 중심 제외, 거리 기반으로 원형에 가깝게
                if dx == 0 && dy == 0 {
                    continue;
                }
                // 맨해튼 거리가 아닌 유클리드 거리로 원형 외곽선 근사
                let dist_sq = dx * dx + dy * dy;
                if dist_sq <= thickness * thickness {
                    unsafe {
                        let _ = TextOutW(self.hdc_mem, x + dx, y + dy, text);
                    }
                }
            }
        }
    }

    /// 텍스트 색상 설정 (ARGB -> GDI COLORREF)
    fn set_text_color(&self, color: u32) {
        let r = ((color >> 16) & 0xFF) as u8;
        let g = ((color >> 8) & 0xFF) as u8;
        let b = (color & 0xFF) as u8;
        unsafe {
            SetTextColor(self.hdc_mem, COLORREF(((b as u32) << 16) | ((g as u32) << 8) | (r as u32)));
        }
    }

    /// 텍스트 영역에 알파 적용 (premultiplied alpha) - 전체 텍스트용
    fn apply_text_alpha_full(&mut self) {
        unsafe {
            let pixel_count = (self.width * self.height) as usize;
            let pixels = std::slice::from_raw_parts_mut(self.bits as *mut u32, pixel_count);

            for pixel in pixels.iter_mut() {
                let current_alpha = (*pixel >> 24) & 0xFF;
                // 알파가 이미 설정되지 않은 픽셀에 대해
                if current_alpha == 0 {
                    let r = (*pixel >> 16) & 0xFF;
                    let g = (*pixel >> 8) & 0xFF;
                    let b = *pixel & 0xFF;

                    // 텍스트 색상이 있는 픽셀에 완전 불투명 알파 적용
                    if r > 0 || g > 0 || b > 0 {
                        *pixel = (0xFF << 24) | (r << 16) | (g << 8) | b;
                    }
                }
            }
        }
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
