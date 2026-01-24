//! 스크린샷 캡처 모듈
//!
//! GDI 기반 화면 캡처 및 이미지 저장 기능.
//! - 윈도우 캡처
//! - 화면 영역 캡처
//! - 영역 선택 UI
//! - PNG/JPEG/WebP 저장

use std::mem::zeroed;
use std::path::{Path, PathBuf};

use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE},
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::config::ScreenshotConfig;

/// 스크린샷 결과
pub struct ScreenshotResult {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>, // BGRA 포맷
}

/// 화면 영역 캡처
pub fn capture_screen_region(x: i32, y: i32, width: i32, height: i32) -> Result<ScreenshotResult> {
    unsafe {
        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return Err(Error::from_hresult(HRESULT::from_win32(GetLastError().0)));
        }

        let result = capture_from_dc(hdc_screen, x, y, width, height);

        ReleaseDC(None, hdc_screen);
        result
    }
}

/// 윈도우 캡처
pub fn capture_window(hwnd: HWND, include_frame: bool) -> Result<ScreenshotResult> {
    unsafe {
        let rect = if include_frame {
            let mut r: RECT = zeroed();
            GetWindowRect(hwnd, &mut r)?;
            r
        } else {
            let mut r: RECT = zeroed();
            GetClientRect(hwnd, &mut r)?;
            let mut pt = POINT { x: 0, y: 0 };
            let _ = ClientToScreen(hwnd, &mut pt);
            RECT {
                left: pt.x,
                top: pt.y,
                right: pt.x + r.right,
                bottom: pt.y + r.bottom,
            }
        };

        capture_screen_region(
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        )
    }
}

/// DC에서 캡처
unsafe fn capture_from_dc(
    hdc_src: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<ScreenshotResult> {
    unsafe {
        // 메모리 DC 생성
        let hdc_mem = CreateCompatibleDC(Some(hdc_src));
        if hdc_mem.is_invalid() {
            return Err(Error::from_hresult(HRESULT::from_win32(GetLastError().0)));
        }

        // 호환 비트맵 생성
        let hbitmap: HBITMAP = CreateCompatibleBitmap(hdc_src, width, height);
        if hbitmap.is_invalid() {
            let _ = DeleteDC(hdc_mem);
            return Err(Error::from_hresult(HRESULT::from_win32(GetLastError().0)));
        }

        // 비트맵 선택
        let old_bitmap = SelectObject(hdc_mem, hbitmap.into());

        // 화면 복사
        BitBlt(hdc_mem, 0, 0, width, height, Some(hdc_src), x, y, SRCCOPY)?;

        // 비트맵 데이터 추출
        let mut bmi: BITMAPINFO = zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = width;
        bmi.bmiHeader.biHeight = -height; // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;

        let pixel_count = (width * height) as usize;
        let mut pixels = vec![0u8; pixel_count * 4];

        GetDIBits(
            hdc_mem,
            hbitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        // 정리
        SelectObject(hdc_mem, old_bitmap);
        let _ = DeleteObject(hbitmap.into());
        let _ = DeleteDC(hdc_mem);

        Ok(ScreenshotResult {
            width: width as u32,
            height: height as u32,
            pixels,
        })
    }
}

/// 스크린샷 저장
pub fn save_screenshot(
    result: &ScreenshotResult,
    path: &Path,
    config: &ScreenshotConfig,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    use image::{ImageBuffer, Rgba};

    // BGRA -> RGBA 변환
    let mut rgba_pixels = result.pixels.clone();
    for chunk in rgba_pixels.chunks_exact_mut(4) {
        chunk.swap(0, 2); // B <-> R
        chunk[3] = 255; // 알파 채널 강제 불투명
    }

    let img: ImageBuffer<Rgba<u8>, _> =
        ImageBuffer::from_raw(result.width, result.height, rgba_pixels)
            .ok_or("Failed to create image buffer")?;

    // 포맷에 따라 저장
    match config.format {
        0 => {
            // PNG
            img.save_with_format(path, image::ImageFormat::Png)?;
        }
        1 => {
            // JPEG
            let rgb_img = image::DynamicImage::ImageRgba8(img).to_rgb8();
            let file = std::fs::File::create(path)?;
            let mut encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(file, config.jpeg_quality);
            encoder.encode_image(&rgb_img)?;
        }
        2 => {
            // WebP
            img.save_with_format(path, image::ImageFormat::WebP)?;
        }
        _ => {
            // 기본 PNG
            img.save_with_format(path, image::ImageFormat::Png)?;
        }
    }

    Ok(())
}

/// 타임스탬프 파일명 생성
pub fn generate_filename(config: &ScreenshotConfig) -> PathBuf {
    use chrono::Local;

    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let extension = match config.format {
        0 => "png",
        1 => "jpg",
        2 => "webp",
        _ => "png",
    };

    let base_path = if config.path.is_empty() {
        // 기본 경로: 현재 디렉토리
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        PathBuf::from(&config.path)
    };

    // 디렉토리 생성
    let _ = std::fs::create_dir_all(&base_path);

    base_path.join(format!("Screenshot_{}.{}", timestamp, extension))
}

// ============================================================================
// 영역 선택 UI
// ============================================================================

/// 영역 선택 상태
struct RegionSelectState {
    start_point: POINT,
    current_point: POINT,
    is_selecting: bool,
    result_rect: Option<RECT>,
    cancelled: bool,
}

thread_local! {
    static REGION_SELECT_STATE: std::cell::RefCell<Option<RegionSelectState>> =
        const { std::cell::RefCell::new(None) };
}

const REGION_SELECT_CLASS: PCWSTR = w!("AnemoneRegionSelectClass");

/// 대화형 영역 선택 캡처
pub fn capture_region_interactive() -> Result<ScreenshotResult> {
    unsafe {
        let instance = GetModuleHandleW(None)?;

        // 윈도우 클래스 등록
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(region_select_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance.into(),
            hIcon: HICON::default(),
            hCursor: LoadCursorW(None, IDC_CROSS)?,
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: REGION_SELECT_CLASS,
            hIconSm: HICON::default(),
        };

        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            let err = GetLastError();
            if err != ERROR_CLASS_ALREADY_EXISTS {
                return Err(Error::from_hresult(HRESULT::from_win32(err.0)));
            }
        }

        // 전체 화면 크기
        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let screen_height = GetSystemMetrics(SM_CYSCREEN);

        // 상태 초기화
        REGION_SELECT_STATE.with(|cell| {
            *cell.borrow_mut() = Some(RegionSelectState {
                start_point: POINT { x: 0, y: 0 },
                current_point: POINT { x: 0, y: 0 },
                is_selecting: false,
                result_rect: None,
                cancelled: false,
            });
        });

        // 전체 화면 오버레이 윈도우 생성
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            REGION_SELECT_CLASS,
            w!(""),
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            screen_width,
            screen_height,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        // 반투명 설정
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 150, LWA_ALPHA)?;

        // 메시지 루프
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);

            if !IsWindow(Some(hwnd)).as_bool() {
                break;
            }
        }

        // 결과 가져오기
        let (rect, cancelled) = REGION_SELECT_STATE.with(|cell| {
            let state = cell.borrow();
            if let Some(ref s) = *state {
                (s.result_rect, s.cancelled)
            } else {
                (None, true)
            }
        });

        // 상태 정리
        REGION_SELECT_STATE.with(|cell| {
            *cell.borrow_mut() = None;
        });

        if cancelled {
            return Err(Error::from_hresult(HRESULT::from_win32(ERROR_CANCELLED.0)));
        }

        let rect =
            rect.ok_or_else(|| Error::from_hresult(HRESULT::from_win32(ERROR_CANCELLED.0)))?;

        // 선택된 영역 캡처
        capture_screen_region(
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        )
    }
}

/// RECT 정규화 (left < right, top < bottom 보장)
fn normalize_rect(p1: POINT, p2: POINT) -> RECT {
    RECT {
        left: p1.x.min(p2.x),
        top: p1.y.min(p2.y),
        right: p1.x.max(p2.x),
        bottom: p1.y.max(p2.y),
    }
}

/// 영역 선택 WndProc
unsafe extern "system" fn region_select_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_LBUTTONDOWN => {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

                REGION_SELECT_STATE.with(|cell| {
                    if let Some(ref mut state) = *cell.borrow_mut() {
                        state.start_point = POINT { x, y };
                        state.current_point = POINT { x, y };
                        state.is_selecting = true;
                        let _ = SetCapture(hwnd);
                    }
                });
                LRESULT(0)
            }

            WM_MOUSEMOVE => {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

                REGION_SELECT_STATE.with(|cell| {
                    if let Some(ref mut state) = *cell.borrow_mut() {
                        if state.is_selecting {
                            state.current_point = POINT { x, y };
                            let _ = InvalidateRect(Some(hwnd), None, true);
                        }
                    }
                });
                LRESULT(0)
            }

            WM_LBUTTONUP => {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

                REGION_SELECT_STATE.with(|cell| {
                    if let Some(ref mut state) = *cell.borrow_mut() {
                        if state.is_selecting {
                            state.current_point = POINT { x, y };
                            state.result_rect =
                                Some(normalize_rect(state.start_point, state.current_point));
                            state.is_selecting = false;
                            let _ = ReleaseCapture();
                        }
                    }
                });
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }

            WM_KEYDOWN => {
                if wparam.0 == VK_ESCAPE.0 as usize {
                    REGION_SELECT_STATE.with(|cell| {
                        if let Some(ref mut state) = *cell.borrow_mut() {
                            state.cancelled = true;
                            state.is_selecting = false;
                        }
                    });
                    let _ = ReleaseCapture();
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }

            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);

                // 반투명 검은색 배경
                let mut rc: RECT = zeroed();
                let _ = GetClientRect(hwnd, &mut rc);
                let brush = CreateSolidBrush(COLORREF(0x282828));
                FillRect(hdc, &rc, brush);
                let _ = DeleteObject(brush.into());

                // 선택 영역 그리기
                REGION_SELECT_STATE.with(|cell| {
                    if let Some(ref state) = *cell.borrow() {
                        if state.is_selecting {
                            let rect = normalize_rect(state.start_point, state.current_point);

                            // 빨간색 테두리
                            let pen = CreatePen(PS_SOLID, 2, COLORREF(0x0000FF));
                            let old_pen = SelectObject(hdc, pen.into());
                            let null_brush = GetStockObject(NULL_BRUSH);
                            let old_brush = SelectObject(hdc, null_brush);

                            Rectangle(hdc, rect.left, rect.top, rect.right, rect.bottom);

                            SelectObject(hdc, old_brush);
                            SelectObject(hdc, old_pen);
                            let _ = DeleteObject(pen.into());

                            // 크기 텍스트
                            let width = rect.right - rect.left;
                            let height = rect.bottom - rect.top;
                            let size_text = format!("{}x{}", width, height);
                            let size_wide: Vec<u16> =
                                size_text.encode_utf16().chain(std::iter::once(0)).collect();

                            SetTextColor(hdc, COLORREF(0x00FF00));
                            SetBkMode(hdc, TRANSPARENT);
                            TextOutW(hdc, rect.left + 5, rect.top + 5, &size_wide);
                        }
                    }
                });

                // 안내 텍스트
                let help_text = "드래그하여 영역을 선택하세요. ESC로 취소합니다.";
                let help_wide: Vec<u16> =
                    help_text.encode_utf16().chain(std::iter::once(0)).collect();
                SetTextColor(hdc, COLORREF(0xFFFFFF));
                SetBkMode(hdc, TRANSPARENT);
                TextOutW(hdc, 20, 20, &help_wide);

                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }

            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// 스크린샷 촬영 및 저장 (편의 함수)
pub fn take_screenshot_and_save(
    config: &ScreenshotConfig,
    mode: CaptureMode,
) -> std::result::Result<PathBuf, Box<dyn std::error::Error>> {
    let result = match mode {
        CaptureMode::FullScreen => {
            let width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
            let height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
            capture_screen_region(0, 0, width, height)?
        }
        CaptureMode::Window(hwnd) => capture_window(hwnd, true)?,
        CaptureMode::Region(rect) => capture_screen_region(
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        )?,
        CaptureMode::Interactive => capture_region_interactive()?,
    };

    let path = generate_filename(config);
    save_screenshot(&result, &path, config)?;

    Ok(path)
}

/// 캡처 모드
pub enum CaptureMode {
    FullScreen,
    Window(HWND),
    Region(RECT),
    Interactive,
}
