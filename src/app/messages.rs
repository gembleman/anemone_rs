//! 프로세스 내 Win32 창들이 공유하는 사용자 정의 메시지 ID.

use windows::Win32::UI::WindowsAndMessaging::{WM_APP, WM_USER};

pub(crate) const WM_TRAY_ICON: u32 = WM_USER + 1;
pub(crate) const WM_TRANSLATION_COMPLETE: u32 = WM_USER + 100;
pub(crate) const WM_APP_REFRESH: u32 = WM_APP + 1;
pub(crate) const WM_APP_SET_MAGNETIC: u32 = WM_APP + 2;
pub(crate) const WM_DEFERRED_RESIZE: u32 = WM_APP + 3;
pub(crate) const WM_DEFERRED_PAINT: u32 = WM_APP + 4;
pub(crate) const WM_APP_ACTION: u32 = WM_APP + 5;
pub(crate) const WM_APP_MAGNETIC_TARGET_SELECTED: u32 = WM_APP + 6;
/// 업데이트 워커가 확인·다운로드 결과를 다 채운 뒤 게시하는 알림.
pub(crate) const WM_UPDATE_RESULT: u32 = WM_APP + 40;
/// 다운로드 진행도가 갱신됐을 때 게시하는 알림. 결과와 분리해, 진행도만 다시
/// 그릴 때 결과 큐를 건드리지 않게 한다.
pub(crate) const WM_UPDATE_PROGRESS: u32 = WM_APP + 41;
/// 후킹 워커의 이벤트 슬롯에 결과가 쌓였을 때 게시하는 알림.
pub(crate) const WM_APP_HOOK_STATE: u32 = WM_APP + 7;
