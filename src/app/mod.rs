use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use windows::{
    Win32::Foundation::{HWND, RECT},
    core::{PCWSTR, w},
};

use crate::clipboard::ClipboardWatcher;
use crate::d2d::{CompositionRenderer, D2DRenderer};
use crate::hotkey::HotkeyManager;
use crate::magnetic::MagneticManager;
use crate::menu::ContextMenu;
use crate::services::AppServices;
use crate::tray::TrayIcon;

#[cfg(feature = "benchmark")]
#[path = "../../benchmark/bench.rs"]
mod bench;
#[cfg(feature = "benchmark")]
#[path = "../../benchmark/app.rs"]
mod bench_app;

pub(crate) mod action;
mod commands;
mod lifecycle;
mod rendering;
mod state;
mod translation;
mod window_proc;

const CLASS_NAME: PCWSTR = w!("AnemoneWindowClass");
const PARENT_CLASS_NAME: PCWSTR = w!("AnemoneParentClass");
const WINDOW_TITLE: PCWSTR = w!("아네모네");
pub(super) const COMPOSITION_RETRY_TIMER: usize = 0xD2D0;
pub(super) const CLIPBOARD_DEBOUNCE_TIMER: usize = 0xD2D1;
pub(super) const CLIPBOARD_READ_RETRY_TIMER: usize = 0xD2D2;

pub struct App {
    hwnd: HWND,
    model: state::AppModel,
    action_queue: Rc<RefCell<VecDeque<state::AppAction>>>,
    services: AppServices,
    tray: TrayIcon,
    menu: ContextMenu,
    hotkey: Option<HotkeyManager>,
    clipboard: ClipboardWatcher,
    /// 수동/파일 번역 창이 살아 있는 동안 clipboard listener를 일시 정지한다.
    translate_dialog_session: Option<u64>,
    file_trans_dialog_session: Option<u64>,
    next_clipboard_pause_session: u64,
    taskbar_created_msg: u32,
    magnetic: Option<MagneticManager>,
    d2d_renderer: Option<D2DRenderer>,
    /// 창이 표시된 뒤 첫 paint에서 만드는 DComp renderer.
    composition: Option<CompositionRenderer>,
    /// D3D/DComp 초기화가 연속 실패할 때 paint마다 전체 스택 생성과 로그를
    /// 반복하지 않도록 timer 기반 backoff를 적용한다.
    composition_init_failures: u32,
    composition_retry_scheduled: bool,
    /// 투명 배경에서 `WM_NCHITTEST`가 사용하는 client 좌표 text 사각형.
    hit_region: Vec<RECT>,
    /// 배경/테두리가 있어 text 사각형과 무관하게 전체 창을 조작할 수 있는지 여부.
    /// `false`이면서 `hit_region`도 비어 있으면 완전히 빈 프레임이므로 입력을 통과시킨다.
    full_hit_region: bool,
}

// 전역 앱 인스턴스 (WndProc에서 접근용)
thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}
