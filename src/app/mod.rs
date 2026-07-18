use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::Foundation::{HWND, RECT},
    core::{PCWSTR, w},
};

use crate::clipboard::ClipboardWatcher;
use crate::config::Config;
use crate::d2d::{CompositionRenderer, D2DRenderer};
use crate::dialogs::BacklogStore;
use crate::hotkey::HotkeyManager;
use crate::magnetic::MagneticManager;
use crate::menu::ContextMenu;
use crate::tray::TrayIcon;

#[cfg(feature = "benchmark")]
#[path = "../../benchmark/bench.rs"]
mod bench;
#[cfg(feature = "benchmark")]
#[path = "../../benchmark/app.rs"]
mod bench_app;

mod commands;
mod lifecycle;
mod rendering;
mod state;
mod translation;
mod window_proc;

const CLASS_NAME: PCWSTR = w!("AnemoneWindowClass");
const PARENT_CLASS_NAME: PCWSTR = w!("AnemoneParentClass");
const WINDOW_TITLE: PCWSTR = w!("아네모네");

pub struct App {
    hwnd: HWND,
    state: state::AppState,
    config: Rc<RefCell<Config>>,
    tray: TrayIcon,
    menu: ContextMenu,
    hotkey: Option<HotkeyManager>,
    clipboard: ClipboardWatcher,
    taskbar_created_msg: u32,
    dialogs: DialogWindows,
    backlog_store: Rc<RefCell<BacklogStore>>,
    magnetic: Option<MagneticManager>,
    d2d_renderer: Option<D2DRenderer>,
    /// DComp 합성 렌더러. lazy init: hwnd 가 보이는 시점 (`ShowWindow` 후)
    /// 의 첫 paint 에서 만든다. client size 가 0 이면 swap chain 생성이
    /// 실패하기 때문.
    composition: Option<CompositionRenderer>,
    /// 텍스트가 차지하는 라인 단위 사각형 (클라이언트 좌표).
    ///
    /// 비어 있으면 `WM_NCHITTEST` 가 윈도우 사각 전체를 `HTCAPTION` 으로
    /// 잡는다 (현재 동작). 비어 있지 않으면 점이 사각형 합집합에 들면
    /// `HTCAPTION`, 아니면 `HTTRANSPARENT`. 채워지는 조건은
    /// `background_visible=false` 이고 `current_text` 가 비어있지 않을 때.
    hit_region: Vec<RECT>,
}

#[derive(Default)]
struct DialogWindows {
    settings: Option<HWND>,
    translate: Option<HWND>,
    backlog: Option<HWND>,
    file_trans: Option<HWND>,
    hook_settings: Option<HWND>,
}

// 전역 앱 인스턴스 (WndProc에서 접근용)
thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}
