use std::cell::RefCell;
use std::mem::zeroed;
use std::ptr::null_mut;
use std::rc::Rc;
use std::sync::Arc;

use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Dxgi::{DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET},
            Gdi::{BeginPaint, ClientToScreen, EndPaint, HBRUSH, PAINTSTRUCT, UpdateWindow},
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::clipboard::ClipboardWatcher;
use crate::config::Config;
use crate::constants::{
    APP_ICON_ID, INITIAL_WINDOW_HEIGHT, INITIAL_WINDOW_WIDTH, INITIAL_WINDOW_X, INITIAL_WINDOW_Y,
    MIN_WINDOW_SIZE, RESIZE_BORDER_WIDTH, WM_APP_REFRESH, WM_DEFERRED_CLIPBOARD,
    WM_TRANSLATION_COMPLETE, WM_TRAY_ICON,
};
use crate::d2d::{CompositionRenderer, D2DRenderer};
use crate::dialogs::helpers::{Dialog, dispatch_resource_dialog_message};
use crate::dialogs::{
    BacklogDialog, FileTransDialog, HookSettingsDialog, LogEntry, SettingsDialog, TranslateDialog,
    add_to_backlog,
};
use crate::hotkey::HotkeyManager;
use crate::magnetic::MagneticManager;
use crate::menu::{self, ContextMenu};
use crate::translation::{request_translation, take_response, unregister_translation_hwnd};
use crate::tray::{self, TrayIcon};
use crate::window::{self, TextRenderStyle};

#[cfg(feature = "benchmark")]
#[path = "../../benchmark/bench.rs"]
mod bench;
#[cfg(feature = "benchmark")]
#[path = "../../benchmark/app.rs"]
mod bench_app;

mod commands;
mod lifecycle;
mod rendering;
mod translation;
mod window_proc;

const CLASS_NAME: PCWSTR = w!("AnemoneWindowClass");
const PARENT_CLASS_NAME: PCWSTR = w!("AnemoneParentClass");
const WINDOW_TITLE: PCWSTR = w!("아네모네");

pub struct App {
    hwnd: HWND,
    width: i32,
    height: i32,
    config: Rc<RefCell<Config>>,
    tray: TrayIcon,
    menu: ContextMenu,
    hotkey: Option<HotkeyManager>,
    clipboard: ClipboardWatcher,
    taskbar_created_msg: u32,
    settings_hwnd: Option<HWND>,
    translate_hwnd: Option<HWND>,
    backlog_hwnd: Option<HWND>,
    file_trans_hwnd: Option<HWND>,
    hook_settings_hwnd: Option<HWND>,
    magnetic: Option<MagneticManager>,
    current_text: String,
    d2d_renderer: Option<D2DRenderer>,
    /// DComp 합성 렌더러. lazy init: hwnd 가 보이는 시점 (`ShowWindow` 후)
    /// 의 첫 paint 에서 만든다. client size 가 0 이면 swap chain 생성이
    /// 실패하기 때문.
    composition: Option<CompositionRenderer>,
    /// 대기 중인 번역의 원문 (번역 완료 시 백로그에 추가)
    pending_original_text: Option<String>,
    /// 텍스트가 차지하는 라인 단위 사각형 (클라이언트 좌표).
    ///
    /// 비어 있으면 `WM_NCHITTEST` 가 윈도우 사각 전체를 `HTCAPTION` 으로
    /// 잡는다 (현재 동작). 비어 있지 않으면 점이 사각형 합집합에 들면
    /// `HTCAPTION`, 아니면 `HTTRANSPARENT`. 채워지는 조건은
    /// `background_visible=false` 이고 `current_text` 가 비어있지 않을 때.
    hit_region: Vec<RECT>,
}

// 전역 앱 인스턴스 (WndProc에서 접근용)
thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}
