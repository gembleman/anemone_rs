//! 텍스트 스트림 수신 반영과 미리보기/선택, 목록 우클릭 메뉴.

use std::cell::Cell;

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    LB_GETITEMRECT, LB_ITEMFROMPOINT, LB_SETCURSEL, SendMessageW,
};

use crate::dialogs::helpers::show_error_message;
use crate::dialogs::host::DialogHost;
use crate::hook::text_bridge::HookText;
use crate::menu::{ContextMenu, enabled_flag};

use super::view::{
    StreamLabel, add_list_string, append_fits, append_log_text, list_selection,
    replace_list_string, set_log_text, stream_label, update_stream_indexed,
};
use super::{
    AUTO_SELECT_HOOK_NAME, HookFindDialog, INSTALLED_HOOK_CODES, SELECTED_SOURCE, set_enabled,
    set_selected_source,
};

/// 우클릭 팝업의 명령 ID. `TPM_RETURNCMD`로 값을 직접 돌려받으므로 다른 메뉴의
/// ID와 겹치지 않아도 된다.
const MENU_COPY_HOOK_CODE: u16 = 1;

impl HookFindDialog {
    pub(super) fn observe_text(&mut self, text: HookText) {
        // 스트림은 나타난 순서대로 뒤에만 붙는다. 항목 인덱스가
        // `self.streams`의 인덱스와 계속 1:1이라 선택이 어긋나지 않는다.
        let source = text.source;
        let previous_chars = self
            .stream_indices
            .get(&source)
            .and_then(|&index| self.streams.get(index))
            .map(|stream| stream.history.chars().count());
        let Some((index, label)) =
            update_stream_indexed(&mut self.streams, &mut self.stream_indices, text.clone())
        else {
            return;
        };
        let stream = &self.streams[index];
        match label {
            StreamLabel::Added => add_list_string(self.streams_list, &stream_label(stream)),
            StreamLabel::Changed => {
                replace_list_string(self.streams_list, index, &stream_label(stream));
            }
            StreamLabel::Unchanged => {}
        }
        if SELECTED_SOURCE.with(Cell::get) == Some(stream.source) {
            if let Some(previous_chars) = previous_chars
                && append_fits(previous_chars, &text.text.replace('\n', "\r\n"))
            {
                let mut delta = String::new();
                if previous_chars != 0 {
                    delta.push_str("\r\n");
                }
                delta.push_str(&text.text.replace('\n', "\r\n"));
                append_log_text(self.preview, &delta);
            } else {
                set_log_text(self.preview, &stream.history);
            }
        }
    }

    pub(super) fn preview_selected_stream(&mut self) {
        let Some(stream) =
            list_selection(self.streams_list).and_then(|index| self.streams.get(index))
        else {
            return;
        };
        set_log_text(self.preview, &stream.history);
        set_enabled(self.stream_select_btn, self.target_label.is_some());
    }

    pub(super) fn select_stream(&mut self) {
        let Some(stream) =
            list_selection(self.streams_list).and_then(|index| self.streams.get(index))
        else {
            return;
        };
        set_selected_source(Some(stream.source));
        AUTO_SELECT_HOOK_NAME.with(|slot| *slot.borrow_mut() = None);
        let hook_code =
            INSTALLED_HOOK_CODES.with(|codes| codes.borrow().get(&stream.source.address).cloned());
        self.actions
            .save_hook_profile(stream.hook_name.clone(), hook_code);
    }
}

/// 스레드 목록의 우클릭 메뉴를 띄운다.
///
/// `TrackPopupMenu`는 자체 message loop를 돌리므로, dialog state를 빌린 채로
/// 부르면 메뉴가 떠 있는 동안 도착한 후킹 텍스트가 `with_state_mut` 실패로
/// 사라진다. 그래서 host가 state를 빌리기 전 단계에서 호출하고, 필요한 값만
/// 그때그때 짧게 빌려 읽는다.
pub(super) fn show_context_menu(dialog: HWND, list: HWND, lparam: LPARAM) {
    let Some((index, x, y)) = context_menu_target(list, lparam) else {
        return;
    };
    // 명령이 어느 스레드에 적용되는지 보이도록 우클릭한 항목으로 선택을 옮긴다.
    // SAFETY: list는 dialog 템플릿에서 얻은 유효한 목록 control이다.
    unsafe {
        let _ = SendMessageW(list, LB_SETCURSEL, index, 0);
    }
    DialogHost::<HookFindDialog>::with_state_mut(HookFindDialog::preview_selected_stream);

    let hook_code = DialogHost::<HookFindDialog>::with_state(|state| {
        state.streams.get(index).map(|stream| stream.source.address)
    })
    .flatten()
    .and_then(|address| INSTALLED_HOOK_CODES.with(|codes| codes.borrow().get(&address).cloned()));

    let menu = match ContextMenu::new() {
        Ok(menu) => menu,
        Err(error) => {
            tracing::warn!("스레드 우클릭 메뉴를 만들지 못했습니다: {error}");
            return;
        }
    };
    // 코드를 아직 받지 못한 스레드(설치 통지 이전이거나 이전 세션의 잔여 항목)는
    // 복사할 것이 없으므로 항목을 비활성화한다.
    if let Err(error) = menu.append(
        enabled_flag(hook_code.is_some()),
        MENU_COPY_HOOK_CODE,
        "후킹 코드 복사",
    ) {
        tracing::warn!("스레드 우클릭 메뉴 항목을 추가하지 못했습니다: {error}");
        return;
    }

    match menu.show(dialog, x, y) {
        Ok(Some(MENU_COPY_HOOK_CODE)) => copy_hook_code(dialog, hook_code),
        Ok(_) => {}
        Err(error) => tracing::warn!("스레드 우클릭 메뉴를 띄우지 못했습니다: {error}"),
    }
}

fn copy_hook_code(dialog: HWND, hook_code: Option<String>) {
    // 항목은 코드를 아는 경우에만 활성화되므로 여기서 None은 오지 않는다.
    let Some(hook_code) = hook_code else {
        return;
    };
    match crate::clipboard::set_text(dialog, &hook_code) {
        Ok(()) => tracing::info!(hook_code = %hook_code, "후크 코드를 클립보드에 복사"),
        Err(error) => {
            tracing::warn!("후크 코드 클립보드 복사 실패: {error}");
            show_error_message(
                dialog,
                "복사 실패",
                "후크 코드를 클립보드에 복사하지 못했습니다. 다른 프로그램이 클립보드를 붙잡고 있는지 확인해 주세요.",
            );
        }
    }
}

/// 우클릭 지점이 가리키는 항목과 메뉴를 띄울 화면 좌표.
///
/// `WM_CONTEXTMENU`의 LPARAM이 -1이면 키보드(Shift+F10, 메뉴 키)로 연 것이라
/// 좌표가 없다. 그때는 현재 선택 항목을 대상으로 삼고 그 항목 바로 아래에 띄운다.
fn context_menu_target(list: HWND, lparam: LPARAM) -> Option<(usize, i32, i32)> {
    if lparam == -1 {
        let index = list_selection(list)?;
        let mut rect = RECT::default();
        // SAFETY: list는 유효한 목록 control이고, rect는 스택 위 유효한 버퍼다.
        if unsafe { SendMessageW(list, LB_GETITEMRECT, index, &mut rect as *mut RECT as isize) } < 0
        {
            return None;
        }
        let mut point = POINT {
            x: rect.left,
            y: rect.bottom,
        };
        // SAFETY: point는 스택 위 유효한 버퍼다.
        if unsafe { ClientToScreen(list, &mut point) } == 0 {
            return None;
        }
        return Some((index, point.x, point.y));
    }

    let x = (lparam & 0xffff) as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as i16 as i32;
    let mut point = POINT { x, y };
    // SAFETY: point는 스택 위 유효한 버퍼이고, list는 유효한 목록 control이다.
    if unsafe { ScreenToClient(list, &mut point) } == 0 {
        return None;
    }
    // SAFETY: LB_ITEMFROMPOINT는 LPARAM에 담긴 client 좌표만 읽는다.
    let hit = unsafe { SendMessageW(list, LB_ITEMFROMPOINT, 0, pack_point(point)) };
    // HIWORD가 0이 아니면 항목 바깥을 눌렀다는 뜻이다(빈 목록 포함).
    if (hit >> 16) & 0xffff != 0 {
        return None;
    }
    Some(((hit & 0xffff) as usize, x, y))
}

/// `LB_ITEMFROMPOINT`가 요구하는 `POINTS`(16비트 쌍) 형태로 client 좌표를 담는다.
fn pack_point(point: POINT) -> isize {
    let x = point.x as u32 & 0xffff;
    let y = point.y as u32 & 0xffff;
    ((y << 16) | x) as isize
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/hook_find/streams.rs"]
mod tests;
