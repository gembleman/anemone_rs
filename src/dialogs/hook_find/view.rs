//! 스트림/후보 목록의 뷰 모델과 표시용 helper.
//!
//! `HookFindDialog`가 들고 있는 스트림 이력·후보 목록의 저장 형태와, 목록
//! control에 항목을 넣거나 텍스트를 읽고 쓰는 저수준 Win32 호출을 모은다.

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::{EM_SCROLLCARET, EM_SETSEL};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetDlgItem, GetWindowTextLengthW, GetWindowTextW, LB_ADDSTRING, LB_DELETESTRING, LB_GETCURSEL,
    LB_INSERTSTRING, LB_SETCURSEL, SendMessageW, SetWindowTextW,
};

use crate::hook::pipe_client::FoundHook;
use crate::hook::text_bridge::{HookSource, HookText};

#[derive(Debug)]
pub(super) struct StreamView {
    pub(super) source: HookSource,
    pub(super) hook_name: String,
    pub(super) history: String,
}

/// [`update_stream`] 뒤에 목록 control에 해야 할 일.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StreamLabel {
    /// 처음 보는 스레드다. 항목을 뒤에 더한다.
    Added,
    /// 후크 이름이 바뀌었다. 같은 자리의 항목 문자열을 갈아 끼운다.
    Changed,
    /// 이력만 늘었다. 항목 문자열은 그대로다.
    Unchanged,
}

#[derive(Debug, Default)]
pub(super) struct SavedView {
    pub(super) streams: Vec<StreamView>,
    pub(super) candidates: Vec<FoundHook>,
    pub(super) installed_candidate: Option<u64>,
    pub(super) installed_manual: Option<u64>,
    pub(super) manual_code: String,
    pub(super) search_text: String,
    pub(super) candidate_selection: Option<usize>,
}

fn display_hook_name(name: &str) -> &str {
    if name.is_empty() { "Unnamed" } else { name }
}

pub(super) fn stream_label(stream: &StreamView) -> String {
    format!(
        "{} [{:X}:{:X}:{:X}]",
        display_hook_name(&stream.hook_name),
        stream.source.address,
        stream.source.context,
        stream.source.subcontext
    )
}

pub(super) fn candidate_label(found: &FoundHook) -> String {
    format!(
        "[{:X}] flags={:X}  {}",
        found.hook_address,
        found.hook_type_flags,
        one_line_preview(&found.text, 64)
    )
}

/// 새 문장을 스트림 이력에 적고, 목록 control에 반영할 것이 있는지 돌려준다.
pub(super) fn update_stream(streams: &mut Vec<StreamView>, text: HookText) -> (usize, StreamLabel) {
    let (index, added) = if let Some(index) = streams
        .iter()
        .position(|stream| stream.source == text.source)
    {
        (index, false)
    } else {
        streams.push(StreamView {
            source: text.source,
            hook_name: text.hook_name.clone(),
            history: String::new(),
        });
        (streams.len() - 1, true)
    };

    let stream = &mut streams[index];
    // 같은 ThreadParam이라도 후크 이름은 바뀔 수 있다 — 저장된 후크 코드를
    // 자동 설치하면 이름이 `UserUI`로 덮인다. 라벨을 그대로 두면 목록이 옛
    // 이름으로 굳어, 사용자가 고를 때 어느 스레드인지 알 수 없다.
    let renamed = !added && stream.hook_name != text.hook_name;
    stream.hook_name = text.hook_name;
    if !stream.history.is_empty() {
        stream.history.push_str("\r\n");
    }
    stream.history.push_str(&text.text.replace('\n', "\r\n"));
    trim_history(&mut stream.history, 16_000);

    let label = match (added, renamed) {
        (true, _) => StreamLabel::Added,
        (false, true) => StreamLabel::Changed,
        (false, false) => StreamLabel::Unchanged,
    };
    (index, label)
}

pub(super) fn add_list_string(list: HWND, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = SendMessageW(list, LB_ADDSTRING, 0, wide.as_ptr() as isize);
    }
}

/// 같은 자리의 항목 문자열만 갈아 끼운다.
///
/// `LB_DELETESTRING`은 지운 항목이 선택돼 있었으면 선택을 없앤다. 순서는
/// 바뀌지 않으므로 인덱스로 되돌려 놓으면 사용자가 고르던 자리를 잃지 않는다.
pub(super) fn replace_list_string(list: HWND, index: usize, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    // SAFETY: list는 dialog 템플릿에서 얻은 유효한 목록 control이고, wide는
    // NUL로 끝나는 유효한 버퍼다.
    unsafe {
        let selected = SendMessageW(list, LB_GETCURSEL, 0, 0);
        let _ = SendMessageW(list, LB_DELETESTRING, index, 0);
        let _ = SendMessageW(list, LB_INSERTSTRING, index, wide.as_ptr() as isize);
        if selected >= 0 {
            let _ = SendMessageW(list, LB_SETCURSEL, selected as usize, 0);
        }
    }
}

pub(super) fn list_selection(list: HWND) -> Option<usize> {
    let selection = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) };
    (selection >= 0).then_some(selection as usize)
}

pub(super) fn read_control_text(dialog: HWND, id: u16) -> String {
    unsafe {
        let control = GetDlgItem(dialog, id as i32);
        if control.is_null() {
            return String::new();
        }
        let len = GetWindowTextLengthW(control) as usize;
        let mut buffer = vec![0u16; len + 1];
        let written = GetWindowTextW(control, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..written.max(0) as usize])
    }
}

pub(super) fn set_control_text(dialog: HWND, id: u16, text: &str) {
    unsafe {
        let control = GetDlgItem(dialog, id as i32);
        if !control.is_null() {
            let wide = crate::win32::to_wide(text);
            let _ = SetWindowTextW(control, wide.as_ptr());
        }
    }
}

pub(super) fn set_log_text(control: HWND, text: &str) {
    let wide = crate::win32::to_wide(text);
    unsafe {
        let _ = SetWindowTextW(control, wide.as_ptr());
        let end = wide.len().saturating_sub(1);
        let _ = SendMessageW(control, EM_SETSEL, end, end as isize);
        let _ = SendMessageW(control, EM_SCROLLCARET, 0, 0);
    }
}

fn one_line_preview(text: &str, max_chars: usize) -> String {
    let mut preview: String = text
        .chars()
        .map(|ch| if ch == '\r' || ch == '\n' { ' ' } else { ch })
        .take(max_chars + 1)
        .collect();
    if preview.chars().count() > max_chars {
        preview = preview.chars().take(max_chars).collect();
        preview.push('…');
    }
    preview
}

fn trim_history(history: &mut String, max_chars: usize) {
    let count = history.chars().count();
    if count > max_chars {
        *history = history.chars().skip(count - max_chars).collect();
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/hook_find/view.rs"]
mod tests;
