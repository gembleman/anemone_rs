use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use windows_sys::Win32::Foundation::{HWND, RECT};

use crate::clipboard::ClipboardWatcher;
use crate::d2d::{CompositionRenderer, D2DRenderer};
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

pub(crate) mod action;
pub(crate) mod backlog;
mod commands;
mod config_save;
pub(crate) mod hook_text;
mod lifecycle;
pub(crate) mod messages;
mod paint;
mod rendering;
pub(crate) mod services;
mod state;
mod translation;
pub(crate) mod translation_cache;
mod update;
mod window_proc;
mod window_proc_handlers;

pub(crate) use update::{open_release_page, restart_after_update_if_requested};

pub(super) const CLASS_NAME: *const u16 = windows_sys::core::w!("AnemoneWindowClass");
pub(super) const PARENT_CLASS_NAME: *const u16 = windows_sys::core::w!("AnemoneParentClass");
pub(super) const WINDOW_TITLE: *const u16 = windows_sys::core::w!("아네모네");
pub(super) const COMPOSITION_RETRY_TIMER: usize = 0xD2D0;
pub(super) const CLIPBOARD_DEBOUNCE_TIMER: usize = 0xD2D1;
pub(super) const CLIPBOARD_READ_RETRY_TIMER: usize = 0xD2D2;
pub(super) const MAGNETIC_NOTICE_TIMER: usize = 0xD2D3;
pub(super) const MAGNETIC_NOTICE_DURATION_MS: u32 = 2_000;
/// 후킹 텍스트 병합 창 타이머.
pub(super) const HOOK_MERGE_TIMER: usize = 0xD2D4;

fn config_hook_merge_ms(config: &crate::config::Config) -> u64 {
    u64::from(config.hook.merge_window_ms).max(1)
}

pub struct App {
    hwnd: HWND,
    model: state::AppModel,
    action_queue: Rc<RefCell<VecDeque<state::AppAction>>>,
    services: services::AppServices,
    tray: TrayIcon,
    menu: ContextMenu,
    hotkey: Option<HotkeyManager>,
    clipboard: ClipboardWatcher,
    /// 수동/파일 번역 창이 살아 있는 동안 clipboard listener를 일시 정지한다.
    translate_dialog_session: Option<u64>,
    file_trans_dialog_session: Option<u64>,
    settings_dialog_active: bool,
    context_menu_active: bool,
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
    /// 후킹 병합 타이머가 지금 돌고 있는지.
    ///
    /// `SetTimer`를 이벤트마다 다시 부르면 같은 ID의 countdown이 매번 리셋되어,
    /// 글자 단위 훅처럼 쉬지 않고 오는 출처가 있으면 타이머가 영영 안 터진다.
    /// 그렇다고 "merger가 비어 있을 때만 건다"로 두면, 그 글자 훅이 merger를
    /// 계속 채우고 있어 시나리오 문장이 타이머 없이 갇힌다. 그래서 merger의
    /// 상태와 무관하게 타이머의 수명만 여기서 따로 추적한다.
    hook_merge_timer_active: bool,
    last_render_diagnostic: Option<String>,
    /// 투명 배경에서 `WM_NCHITTEST`가 사용하는 client 좌표 text 사각형.
    hit_region: Vec<RECT>,
    /// 배경/테두리가 있어 text 사각형과 무관하게 전체 창을 조작할 수 있는지 여부.
    /// `false`이면서 `hit_region`도 비어 있으면 완전히 빈 프레임이므로 입력을 통과시킨다.
    full_hit_region: bool,
    /// 마지막으로 확인해 발견한 업데이트. 사용자가 "업데이트 확인" 버튼으로
    /// 다운로드·적용을 요청할 때 다시 조회하지 않고 이 값을 사용한다.
    pending_update: Option<crate::update::AvailableUpdate>,
    /// 확인/다운로드가 진행 중이라 설정 창의 버튼을 다시 눌러도 무시해야 하는지.
    update_operation_in_progress: bool,
}

// 전역 앱 인스턴스 (WndProc에서 접근용)
thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}
