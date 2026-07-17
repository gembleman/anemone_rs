//! 앱 전역 상수
//!
//! WM_USER 기반 사용자 정의 메시지 ID, Win32 누락 상수,
//! 윈도우 기본값 등을 중앙에서 관리한다.

use windows::Win32::UI::WindowsAndMessaging::{WM_APP, WM_USER};

// ── 메인 윈도우 ─────────────────────────────────────────

/// 실행 파일에 포함된 애플리케이션 아이콘 리소스 ID
pub const APP_ICON_ID: u32 = 1;
/// 메인 윈도우 초기 너비
pub const INITIAL_WINDOW_WIDTH: i32 = 400;
/// 메인 윈도우 초기 높이
pub const INITIAL_WINDOW_HEIGHT: i32 = 200;
/// 메인 윈도우 초기 X 위치
pub const INITIAL_WINDOW_X: i32 = 100;
/// 메인 윈도우 초기 Y 위치
pub const INITIAL_WINDOW_Y: i32 = 100;
/// 윈도우 최소 크기 (너비/높이 공용)
pub const MIN_WINDOW_SIZE: i32 = 100;
/// 테두리 크기 조절 영역 너비 (WM_NCHITTEST)
pub const RESIZE_BORDER_WIDTH: i32 = 8;

// ── 사용자 정의 메시지 (WM_USER) ────────────────────────

/// 트레이 아이콘 콜백 메시지
pub const WM_TRAY_ICON: u32 = WM_USER + 1;

/// 번역 완료 알림 (워커 → UI)
pub const WM_TRANSLATION_COMPLETE: u32 = WM_USER + 100;

/// 지연된 클립보드 처리 (RefCell try_borrow_mut 실패 시 재시도용)
pub const WM_DEFERRED_CLIPBOARD: u32 = WM_USER + 200;

/// 설정/메뉴 변경 후 메인 윈도우 상태 동기화 + repaint 요청
pub const WM_APP_REFRESH: u32 = WM_APP + 1;

// ── 파일 번역 진행률 메시지 ─────────────────────────────

/// 공유 진행률 이벤트 큐 확인 요청

// ── 응답 저장소 ─────────────────────────────────────────

/// 번역 응답 저장소 최대 크기 (초과 시 오래된 항목 제거)
pub const MAX_RESPONSE_STORAGE: usize = 100;

// ── Win32 누락 상수: TrackBar ───────────────────────────

/// `TBM_GETPOS` (= `WM_USER`).
///
/// `windows` 0.62 의 `Win32::UI::Controls` 에 다른 `TBM_*` 상수는 모두 있지만
/// 정작 `TBM_GETPOS` 만 누락되어 직접 정의한다. 상위 windows crate 가 추가하면
/// 이 상수를 제거하고 `windows::Win32::UI::Controls::TBM_GETPOS` 로 교체할 것.
pub const TBM_GETPOS_VAL: u32 = 1024;

// ── Win32 누락 상수: 색상 대화상자 ──────────────────────

/// 표준 CHOOSECOLOR 다이얼로그의 RGB 입력 필드 ID
pub const COLOR_RED_EDIT: u16 = 0x2C2;
pub const COLOR_GREEN_EDIT: u16 = 0x2C3;
pub const COLOR_BLUE_EDIT: u16 = 0x2C4;
