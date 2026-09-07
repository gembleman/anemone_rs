//! RichEdit 본문 갱신: 항목 추가, 스타일 적용, 필터 변경 시 다시 그리기.

use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::format::{CharFormat2W, make_char_format};
use super::{BACKLOG_VIEW_DIRTY, BacklogDialog, drain_pending_entries};
use crate::app::backlog::TextKind;
use crate::win32::to_wide;

// RichEdit messages not exported by windows-sys' Controls module.
const EM_SETCHARFORMAT: u32 = 0x0444;
const SCF_SELECTION: usize = 0x0001;
const EM_SETREDRAW: u32 = 0x000B;

impl BacklogDialog {
    /// RichEdit에 항목 추가
    pub(super) fn append_styled_texts_to_richedit(
        &self,
        segments: impl IntoIterator<Item = crate::app::backlog::StyledText>,
    ) {
        // SAFETY: self.richedit is a valid RichEdit control handle from create_controls.
        // SendMessageW and append_styled_text use valid control handles.
        unsafe {
            let _ = SendMessageW(self.richedit, EM_SETREDRAW, 0, 0);
            let _ = SendMessageW(self.richedit, EM_SETSEL, usize::MAX, -1);

            // COLORREF 는 0x00BBGGRR 순서.
            const COLOR_NAME: u32 = 0x00A00000; // #0000A0 진청색 ([name])
            const COLOR_ORIGINAL: u32 = 0x00000000; // #000000 검정 (원문)
            const COLOR_TRANSLATE: u32 = 0x00008000; // #008000 진녹색 (번역)

            for segment in segments {
                let (color, bold) = match segment.kind {
                    TextKind::Name => (COLOR_NAME, true),
                    TextKind::Original => (COLOR_ORIGINAL, false),
                    TextKind::Translation => (COLOR_TRANSLATE, false),
                };
                self.append_styled_text(&segment.text, color, bold);
            }

            let _ = SendMessageW(self.richedit, EM_SCROLLCARET, 0, 0);
            let _ = SendMessageW(self.richedit, EM_SETREDRAW, 1, 0);
            let _ = InvalidateRect(self.richedit, std::ptr::null(), 0);
        }
    }

    /// RichEdit 본문의 앞에서 UTF-16 문자 수만큼 제거한다.
    pub(super) fn remove_prefix_from_richedit(&self, chars: usize) {
        if chars == 0 {
            return;
        }
        // SAFETY: self.richedit is a valid RichEdit control handle.
        unsafe {
            let _ = SendMessageW(self.richedit, EM_SETREDRAW, 0, 0);
            let _ = SendMessageW(self.richedit, EM_SETSEL, 0, chars as isize);
            let empty = [0u16];
            let _ = SendMessageW(self.richedit, EM_REPLACESEL, 0, empty.as_ptr() as isize);
            let _ = SendMessageW(self.richedit, EM_SETSEL, usize::MAX, -1);
            let _ = SendMessageW(self.richedit, EM_SETREDRAW, 1, 0);
            let _ = InvalidateRect(self.richedit, std::ptr::null(), 0);
        }
    }

    /// 스타일 텍스트 추가
    unsafe fn append_styled_text(&self, text: &str, color: u32, bold: bool) {
        unsafe {
            let wide = to_wide(text);
            if wide.len() <= 1 {
                return;
            }
            let start = GetWindowTextLengthW(self.richedit).max(0) as usize;
            let _ = SendMessageW(self.richedit, EM_REPLACESEL, 0, wide.as_ptr() as isize);
            let end = start + wide.len() - 1;
            let _ = SendMessageW(self.richedit, EM_SETSEL, start, end as isize);
            let format = make_char_format(
                self.font_face.as_deref(),
                self.font_point_size,
                self.font_italic,
                color,
                bold,
            );
            let _ = SendMessageW(
                self.richedit,
                EM_SETCHARFORMAT,
                SCF_SELECTION,
                (&format as *const CharFormat2W).cast::<u8>() as isize,
            );
            // 다음 segment는 다시 끝에 삽입한다.
            let _ = SendMessageW(self.richedit, EM_SETSEL, end, end as isize);
        }
    }

    /// RichEdit 내용 지우기
    pub(super) fn clear_richedit(&mut self) {
        self.store.clear();
        self.rendered_lengths.clear();
        self.actions.clear_backlog();
        // SAFETY: self.richedit is a valid RichEdit control handle.
        unsafe {
            let _ = SetWindowTextW(self.richedit, crate::win32::to_wide("").as_ptr());
        }
    }

    /// RichEdit 다시 그리기 (필터 변경 시)
    pub(super) fn refresh_richedit(&mut self) {
        // SAFETY: self.richedit is a valid RichEdit control handle.
        unsafe {
            let _ = SendMessageW(self.richedit, EM_SETREDRAW, 0, 0);
            let _ = SetWindowTextW(self.richedit, crate::win32::to_wide("").as_ptr());
            let segments = self.store.render(self.filter, self.add_linefeed);
            let _ = SendMessageW(self.richedit, EM_SETSEL, usize::MAX, -1);
            for segment in segments {
                let (color, bold) = match segment.kind {
                    TextKind::Name => (0x00A00000, true),
                    TextKind::Original => (0x00000000, false),
                    TextKind::Translation => (0x00008000, false),
                };
                self.append_styled_text(&segment.text, color, bold);
            }
            let _ = SendMessageW(self.richedit, EM_SCROLLCARET, 0, 0);
            let _ = SendMessageW(self.richedit, EM_SETREDRAW, 1, 0);
            let _ = InvalidateRect(self.richedit, std::ptr::null(), 0);
        }
        self.rendered_lengths = self
            .store
            .render_entry_lengths(self.filter, self.add_linefeed)
            .into_iter()
            .collect();
    }

    pub(super) fn refresh_if_dirty(&mut self) {
        if BACKLOG_VIEW_DIRTY.with(|dirty| dirty.replace(false)) {
            drain_pending_entries(&mut self.store);
            self.refresh_richedit();
        }
    }
}
