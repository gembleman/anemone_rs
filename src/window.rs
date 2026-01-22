use std::mem::zeroed;
use std::ptr::null_mut;

use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
};

const CLASS_NAME: PCWSTR = w!("AnemoneWindowClass");
const WINDOW_TITLE: PCWSTR = w!("아네모네");

pub struct App {
    hwnd: HWND,
    #[allow(dead_code)]
    hwnd_parent: HWND,
    width: i32,
    height: i32,
    buffer: Option<DoubleBuffer>,
}

struct DoubleBuffer {
    hdc_mem: HDC,
    hbitmap: HBITMAP,
    hbitmap_old: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
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
    fn new(hdc: HDC, width: i32, height: i32) -> Result<Self> {
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

    fn clear(&mut self, r: u8, g: u8, b: u8, a: u8) {
        unsafe {
            let pixel_count = (self.width * self.height) as usize;
            let pixels = std::slice::from_raw_parts_mut(self.bits as *mut u32, pixel_count);
            let color = ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            pixels.fill(color);
        }
    }

    fn hdc(&self) -> HDC {
        self.hdc_mem
    }
}

static mut APP_INSTANCE: Option<*mut App> = None;

impl App {
    pub fn run() -> Result<()> {
        unsafe {
            let instance = GetModuleHandleW(None)?;

            // Register parent window class (hidden)
            Self::register_class(instance, w!("AnemoneParentClass"), Some(Self::parent_wndproc))?;

            // Register main window class
            Self::register_class(instance, CLASS_NAME, Some(Self::wndproc))?;

            // Create parent window (for layered window support)
            let hwnd_parent = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                w!("AnemoneParentClass"),
                WINDOW_TITLE,
                WS_POPUP,
                0, 0, 0, 0,
                None,
                None,
                Some(instance.into()),
                None,
            )?;

            // Create main layered window
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                WINDOW_TITLE,
                WS_POPUP,
                100, 100, 400, 200,
                Some(hwnd_parent),
                None,
                Some(instance.into()),
                None,
            )?;

            let mut app = Box::new(App {
                hwnd,
                hwnd_parent,
                width: 400,
                height: 200,
                buffer: None,
            });

            APP_INSTANCE = Some(app.as_mut() as *mut App);

            // Initialize double buffer
            let hdc = GetDC(Some(hwnd));
            app.buffer = Some(DoubleBuffer::new(hdc, 400, 200)?);
            ReleaseDC(Some(hwnd), hdc);

            // Initial paint
            app.paint()?;

            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);

            // Message loop
            let mut msg: MSG = zeroed();
            while GetMessageW(&mut msg, None, 0, 0).into() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            APP_INSTANCE = None;
            drop(app);

            Ok(())
        }
    }

    fn register_class(
        instance: HMODULE,
        class_name: PCWSTR,
        wndproc: WNDPROC,
    ) -> Result<()> {
        unsafe {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: wndproc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance.into(),
                hIcon: LoadIconW(None, IDI_APPLICATION)?,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hbrBackground: HBRUSH(null_mut()),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: class_name,
                hIconSm: HICON::default(),
            };

            let atom = RegisterClassExW(&wc);
            if atom == 0 {
                return Err(Error::from_hresult(HRESULT::from_win32(GetLastError().0)));
            }
            Ok(())
        }
    }

    fn paint(&mut self) -> Result<()> {
        unsafe {
            let buffer = match &mut self.buffer {
                Some(b) => b,
                None => return Ok(()),
            };

            // Clear with semi-transparent dark background
            buffer.clear(40, 40, 40, 200);

            // Draw a simple border using GDI
            let pen = CreatePen(PS_SOLID, 2, COLORREF(0x00AAAAAA));
            let old_pen = SelectObject(buffer.hdc(), pen.into());
            let old_brush = SelectObject(buffer.hdc(), GetStockObject(NULL_BRUSH));

            let _ = Rectangle(buffer.hdc(), 0, 0, self.width, self.height);

            SelectObject(buffer.hdc(), old_pen);
            SelectObject(buffer.hdc(), old_brush);
            let _ = DeleteObject(pen.into());

            // Update layered window
            let hdc_screen = GetDC(None);
            let size = SIZE {
                cx: self.width,
                cy: self.height,
            };
            let pt_src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };

            UpdateLayeredWindow(
                self.hwnd,
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

    fn resize(&mut self, width: i32, height: i32) -> Result<()> {
        if width <= 0 || height <= 0 {
            return Ok(());
        }

        self.width = width;
        self.height = height;

        unsafe {
            let hdc = GetDC(Some(self.hwnd));
            self.buffer = Some(DoubleBuffer::new(hdc, width, height)?);
            ReleaseDC(Some(self.hwnd), hdc);
        }

        self.paint()
    }

    unsafe extern "system" fn parent_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe {
            match msg {
                WM_DESTROY => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                WM_NCHITTEST => {
                    // Allow dragging anywhere on window
                    let result = DefWindowProcW(hwnd, msg, wparam, lparam);
                    if result == LRESULT(HTCLIENT as isize) {
                        return LRESULT(HTCAPTION as isize);
                    }
                    result
                }
                WM_RBUTTONUP => {
                    // Right-click to close
                    if let Some(app) = APP_INSTANCE.and_then(|p| p.as_ref()) {
                        DestroyWindow(app.hwnd).ok();
                    }
                    LRESULT(0)
                }
                WM_SIZE => {
                    let width = (lparam.0 & 0xFFFF) as i32;
                    let height = ((lparam.0 >> 16) & 0xFFFF) as i32;
                    if let Some(app) = APP_INSTANCE.and_then(|p| p.as_mut()) {
                        let _ = app.resize(width, height);
                    }
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
    }
}
