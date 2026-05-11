//! 앱 전역 상수
//!
//! WM_USER 기반 사용자 정의 메시지 ID, Win32 누락 상수,
//! 윈도우 기본값 등을 중앙에서 관리한다.

use windows::Win32::UI::WindowsAndMessaging::WM_USER;

// ── 메인 윈도우 ─────────────────────────────────────────

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
/// 배경 투명 시 최소 알파 (alpha=1, 완전 투명이면 클릭 불가)
pub const TRANSPARENT_ALPHA: u32 = 0x01000000;

// ── 사용자 정의 메시지 (WM_USER) ────────────────────────

/// 트레이 아이콘 콜백 메시지
pub const WM_TRAY_ICON: u32 = WM_USER + 1;

/// 번역 완료 알림 (워커 → UI)
pub const WM_TRANSLATION_COMPLETE: u32 = WM_USER + 100;

/// 지연된 클립보드 처리 (RefCell try_borrow_mut 실패 시 재시도용)
pub const WM_DEFERRED_CLIPBOARD: u32 = WM_USER + 200;

// ── 파일 번역 진행률 메시지 ─────────────────────────────

/// 전체 라인 수
pub const WM_PROGRESS_TOTAL_SIZE: u32 = WM_USER + 300;
/// 전체 파일 수
pub const WM_PROGRESS_TOTAL_COUNT: u32 = WM_USER + 301;
/// 현재 파일 인덱스
pub const WM_PROGRESS_INDEX: u32 = WM_USER + 302;
/// 현재 파일 이름
pub const WM_PROGRESS_NAME: u32 = WM_USER + 303;
/// 현재 파일 라인 수
pub const WM_PROGRESS_LIST_SIZE: u32 = WM_USER + 304;
/// 진행률 업데이트
pub const WM_PROGRESS_UPDATE: u32 = WM_USER + 305;
/// 전체 진행 라인
pub const WM_PROGRESS_CURRENT: u32 = WM_USER + 306;
/// 번역 완료
pub const WM_PROGRESS_COMPLETE: u32 = WM_USER + 307;
/// 에러 발생
pub const WM_PROGRESS_ERROR: u32 = WM_USER + 308;

// ── 응답 저장소 ─────────────────────────────────────────

/// 번역 응답 저장소 최대 크기 (초과 시 오래된 항목 제거)
pub const MAX_RESPONSE_STORAGE: usize = 100;

// ── Win32 누락 상수: ComboBox ───────────────────────────

pub const CB_ADDSTRING: u32 = 0x0143;
pub const CB_GETCURSEL: u32 = 0x0147;
pub const CB_SETCURSEL: u32 = 0x014E;
pub const CB_RESETCONTENT: u32 = 0x014B;
pub const CBN_SELCHANGE: u32 = 1;

// ── Win32 누락 상수: ListBox ────────────────────────────

pub const LB_ADDSTRING: u32 = 0x0180;
pub const LB_INSERTSTRING: u32 = 0x0181;
pub const LB_DELETESTRING: u32 = 0x0182;
pub const LB_SETCURSEL: u32 = 0x0186;
pub const LB_GETCURSEL: u32 = 0x0188;
pub const LB_GETTEXT: u32 = 0x0189;
pub const LB_GETTEXTLEN: u32 = 0x018A;
pub const LB_GETCOUNT: u32 = 0x018B;
pub const LB_ERR: i32 = -1;

pub const LBS_NOTIFY: u32 = 0x0001;
pub const LBS_NOINTEGRALHEIGHT: u32 = 0x0100;

// ── Win32 누락 상수: RichEdit ───────────────────────────

pub const EM_REPLACESEL: u32 = 0x00C2;
pub const EM_SCROLLCARET: u32 = 0x00B7;
pub const EM_SETCHARFORMAT: u32 = WM_USER + 68;
pub const EM_SETBKGNDCOLOR: u32 = WM_USER + 67;

// CHARFORMAT2W 마스크
pub const CFM_BOLD: u32 = 0x00000001;
pub const CFM_COLOR: u32 = 0x40000000;
pub const CFM_SIZE: u32 = 0x80000000;

// CHARFORMAT2W 효과
pub const CFE_BOLD: u32 = 0x00000001;

// 선택 범위
pub const SCF_SELECTION: u32 = 0x0001;

// ── Win32 누락 상수: TrackBar ───────────────────────────

/// `TBM_GETPOS` (= `WM_USER`).
///
/// `windows` 0.62 의 `Win32::UI::Controls` 에 다른 `TBM_*` 상수는 모두 있지만
/// 정작 `TBM_GETPOS` 만 누락되어 직접 정의한다. 상위 windows crate 가 추가하면
/// 이 상수를 제거하고 `windows::Win32::UI::Controls::TBM_GETPOS` 로 교체할 것.
pub const TBM_GETPOS_VAL: u32 = 1024;

// ── Win32 누락 상수: Static ─────────────────────────────

pub const SS_LEFT: u32 = 0x00000000;

// ── Win32 누락 상수: Tab Control ────────────────────────
//
// `TCM_INSERTITEMW`, `TCM_GETCURSEL` 는 `windows` crate 의
// `Win32::UI::Controls` 에서 직접 노출되므로 여기서는 정의하지 않는다.
// `TCN_FIRST`/`TCN_SELCHANGE` 는 i32 부호 통지 코드라 별도 유지.

pub const TCN_FIRST: i32 = -550;
pub const TCN_SELCHANGE: i32 = TCN_FIRST - 1;

// ── Win32 누락 상수: Clipboard ──────────────────────────

pub const CF_UNICODETEXT: u32 = 13;

// ── Win32 누락 상수: WinEvent ───────────────────────────

pub const EVENT_SYSTEM_MINIMIZESTART: u32 = 0x0016;
pub const EVENT_SYSTEM_MINIMIZEEND: u32 = 0x0017;
pub const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
pub const EVENT_OBJECT_LOCATIONCHANGE: u32 = 0x800B;
pub const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
pub const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;

// ── Win32 누락 상수: 색상 대화상자 ──────────────────────

/// 표준 CHOOSECOLOR 다이얼로그의 RGB 입력 필드 ID
pub const COLOR_RED_EDIT: u16 = 0x2C2;
pub const COLOR_GREEN_EDIT: u16 = 0x2C3;
pub const COLOR_BLUE_EDIT: u16 = 0x2C4;
