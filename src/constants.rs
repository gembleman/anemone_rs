//! 사용자 message ID, 빠진 Win32 상수와 창 기본값.

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

/// 설정/메뉴 변경 후 메인 윈도우 상태 동기화 + repaint 요청
pub const WM_APP_REFRESH: u32 = WM_APP + 1;

/// 설정 창에서 요청한 자석 모드 상태 (`WPARAM`: 0/1)
pub const WM_APP_SET_MAGNETIC: u32 = WM_APP + 2;

/// 재진입 중 포인터 수명을 연장할 수 없는 DPI/paint 후속 작업
pub const WM_DEFERRED_RESIZE: u32 = WM_APP + 3;
pub const WM_DEFERRED_PAINT: u32 = WM_APP + 4;

/// 다이얼로그가 UI thread의 AppAction queue에 새 작업을 넣었음을 알린다.
pub const WM_APP_ACTION: u32 = WM_APP + 5;

/// 자석 선택 훅이 사용자가 활성화한 외부 창을 주 창에 전달한다 (`WPARAM`: HWND).
pub const WM_APP_MAGNETIC_TARGET_SELECTED: u32 = WM_APP + 6;

// ── 파일 번역 진행률 메시지 ─────────────────────────────

// ── 응답 저장소 ─────────────────────────────────────────

/// 번역 응답 저장소 최대 크기 (초과 시 오래된 항목 제거)
pub const MAX_RESPONSE_STORAGE: usize = 100;

// ── Win32 누락 상수: TrackBar ───────────────────────────

/// `windows` 0.62에 빠진 `TBM_GETPOS` (`WM_USER`).
pub const TBM_GETPOS_VAL: u32 = 1024;

// ── Win32 누락 상수: 색상 대화상자 ──────────────────────

/// 표준 CHOOSECOLOR 다이얼로그의 RGB 입력 필드 ID
pub const COLOR_RED_EDIT: u16 = 0x2C2;
pub const COLOR_GREEN_EDIT: u16 = 0x2C3;
pub const COLOR_BLUE_EDIT: u16 = 0x2C4;
