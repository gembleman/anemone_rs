//! 탭 전환, 번역 엔진별 패널 높이, 저해상도 대응 세로 스크롤.

use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, UI::Controls::*, UI::WindowsAndMessaging::*,
};
use windows_sys::core::BOOL;

use super::{SettingsDialog, TAB_TRANSLATION, ctrl_id};
use crate::translation::TranslationEngine;

/// `EnumChildWindows` 콜백: 자식 핸들을 `LPARAM`이 가리키는 Vec에 모은다.
///
/// 열거 중에는 창을 옮기지 않는다. `SetWindowPos`를 콜백 안에서 호출하면
/// 열거 순서가 흐트러져 일부 컨트롤을 건너뛸 수 있기 때문이다.
unsafe extern "system" fn collect_direct_child(child: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam은 호출부가 넘긴 유효한 Vec<HWND> 포인터이며, 열거가
    // 끝날 때까지 살아 있다.
    unsafe {
        if let Some(children) = (lparam as *mut Vec<HWND>).as_mut() {
            children.push(child);
        }
    }
    TRUE
}

impl SettingsDialog {
    // 리소스 컨트롤의 좌우 여백을 같게 맞춘 96 DPI 디자인 폭.
    // 높이는 선택한 탭에 따라 동적으로 바뀐다.
    pub(super) const WIDTH: i32 = 492;
    // 가로 컨트롤은 고정 배치이므로 잘리지 않는 최소 client 폭을 유지한다.
    pub(super) const MIN_HEIGHT: i32 = 180;

    /// 탭 전환: 현재 탭 컨트롤 숨기고 새 탭 컨트롤 표시
    pub(super) fn switch_tab(&mut self, new_tab: usize) {
        if new_tab == self.current_tab || new_tab >= self.tab_controls.len() {
            return;
        }
        // 번역 탭은 선택된 엔진의 컨트롤만 보여야 하므로, 먼저 그룹박스를 엔진 높이에
        // 맞춘 뒤 표시한다. 전부 SW_SHOW 했다가 되숨기면 비활성 엔진 패널이 깜빡인다.
        let engine = if new_tab == TAB_TRANSLATION {
            self.draft.borrow().translation.get_engine().ok()
        } else {
            None
        };
        if let Some(engine) = engine {
            self.resize_translation_group(engine);
        }

        // SAFETY: All HWNDs in tab_controls are valid child window handles.
        unsafe {
            for &hwnd in &self.tab_controls[self.current_tab] {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            for &hwnd in &self.tab_controls[new_tab] {
                // 번역 엔진 전용 컨트롤은 아래 update_engine_controls가 필요한
                // 그룹만 표시한다. 여기서 전부 표시하면 곧바로 다시 숨기게 된다.
                if new_tab == TAB_TRANSLATION
                    && self
                        .engine_controls
                        .iter()
                        .any(|group| group.contains(&hwnd))
                {
                    continue;
                }
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
        }
        self.current_tab = new_tab;
        // 엔진 컨트롤 중에는 tab_controls에 없는 것(EzTrans 경고 라벨 등)이 있어
        // 위의 hide 루프로는 숨겨지지 않는다. 번역 탭을 벗어날 때도 반드시 호출한다.
        if let Ok(engine) = self.draft.borrow().translation.get_engine() {
            self.update_engine_controls(engine);
        }
        self.adjust_dialog_size_for_tab(new_tab);
    }

    pub(super) fn translation_height_for_engine(engine: TranslationEngine) -> i32 {
        match engine {
            TranslationEngine::EzTrans => 312,
            TranslationEngine::Google | TranslationEngine::Papago | TranslationEngine::Custom => {
                245
            }
            TranslationEngine::MysTranslater => 300,
            TranslationEngine::DeepL => 365,
            TranslationEngine::Llm => 490,
        }
    }

    /// 선택된 엔진의 전용 입력을 감싸도록 번역 설정 그룹박스 높이를 반환한다.
    pub(super) fn translation_group_height_for_engine(engine: TranslationEngine) -> i32 {
        match engine {
            TranslationEngine::DeepL => 136,
            TranslationEngine::Llm => 204,
            TranslationEngine::EzTrans => 110,
            TranslationEngine::Google | TranslationEngine::Papago | TranslationEngine::Custom => 69,
            TranslationEngine::MysTranslater => 100,
        }
    }

    /// 리소스의 번역 설정 그룹박스 상단 y (dialog unit).
    #[cfg(test)]
    const TRANSLATION_GROUP_TOP_DLU: i32 = 19;
    /// 9pt 맑은 고딕 기준 세로 dialog unit → 96 DPI 픽셀 환산 계수(×1000).
    #[cfg(test)]
    const DLU_Y_PER_PX_MILLI: i32 = 1815;

    /// 그룹박스 아래 테두리와 탭 컨트롤 바닥 사이에 남는 세로 여백(96 DPI 픽셀).
    ///
    /// 그룹박스는 dialog unit으로, 패널 높이는 픽셀로 적혀 있어 한쪽만 고치면
    /// 테두리가 탭 밖으로 나가거나 바닥선에 달라붙는다. 두 단위를 한자리에서
    /// 이어 붙여 엔진마다 같은 여백을 유지하는지 테스트가 확인한다.
    #[cfg(test)]
    pub(super) fn translation_group_bottom_margin(engine: TranslationEngine) -> i32 {
        let group_bottom_dlu =
            Self::TRANSLATION_GROUP_TOP_DLU + Self::translation_group_height_for_engine(engine);
        let group_bottom_px = group_bottom_dlu * Self::DLU_Y_PER_PX_MILLI / 1000;
        // layout_for_current_size가 탭 컨트롤을 y=5에 두고 높이를 -70 하므로
        // 탭의 아래 테두리는 항상 패널 높이에서 65픽셀 위에 있다.
        Self::translation_height_for_engine(engine) - 65 - group_bottom_px
    }

    pub(super) fn resize_translation_group(&self, engine: TranslationEngine) {
        let group = unsafe { GetDlgItem(self.hwnd, ctrl_id::TRANSLATION_GROUP as i32) };
        if group.is_null() {
            return;
        }
        let mut current = RECT::default();
        if unsafe { GetWindowRect(group, &mut current) } == 0 {
            return;
        }
        let mut size = RECT {
            bottom: Self::translation_group_height_for_engine(engine),
            ..Default::default()
        };
        unsafe {
            let _ = MapDialogRect(self.hwnd, &mut size);
            let _ = SetWindowPos(
                group,
                std::ptr::null_mut(),
                0,
                0,
                current.right - current.left,
                size.bottom,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    pub(super) fn target_height_for_tab(&self, tab: usize) -> i32 {
        match tab {
            super::TAB_APPEARANCE => 505,
            // 윈도우 옵션 그룹의 마지막 줄(소스 언어 방어 체크박스)까지 감싼다.
            super::TAB_DISPLAY => 367,
            TAB_TRANSLATION => self
                .draft
                .borrow()
                .translation
                .get_engine()
                .map_or(245, Self::translation_height_for_engine),
            super::TAB_HOTKEYS => 360,
            // 업데이트와 스페셜 땡스 UI가 들어간 정보 그룹박스 높이에 맞춘다.
            super::TAB_INFO => 487,
            _ => 505,
        }
    }

    /// 탭에 따라 다이얼로그 클라이언트 높이를 조정 (빈 공간 최소화)
    pub(super) fn adjust_dialog_size_for_tab(&mut self, tab: usize) {
        // 각 탭의 마지막 group 아래에 닫기 button이 오도록 높이를 잡는다.
        let target_height = self.target_height_for_tab(tab);
        // SAFETY: self.hwnd is valid. SetWindowPos uses valid parameters.
        unsafe {
            // scroll_max는 아직 이전 탭 기준이라 scroll_to(0)은 clamp에 걸려
            // 오프셋을 되돌리지 못할 수 있다. 자식 위치를 직접 원점으로 되돌린다.
            self.reset_scroll_offset();
            let monitor = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let work = if GetMonitorInfoW(monitor, &mut info) != 0 {
                info.rcWork
            } else {
                RECT {
                    left: 0,
                    top: 0,
                    right: GetSystemMetrics(SM_CXSCREEN),
                    bottom: GetSystemMetrics(SM_CYSCREEN),
                }
            };
            let work_width = work.right - work.left;
            let work_height = work.bottom - work.top;

            // Client 디자인 크기를 title/border를 포함한 window 크기로 바꾼다.
            let (_, desired_height) = crate::dialogs::helpers::design_to_window_size(
                self.hwnd,
                Self::WIDTH,
                target_height,
            );
            let needs_scroll = desired_height > work_height;
            let style = GetWindowLongPtrW(self.hwnd, GWL_STYLE) as u32;
            let new_style = if needs_scroll {
                style | WS_VSCROLL
            } else {
                style & !WS_VSCROLL
            };
            if style != new_style {
                let _ = SetWindowLongPtrW(self.hwnd, GWL_STYLE, new_style as _);
            }

            // Scrollbar가 client 폭을 줄이지 않도록 전체 크기를 다시 계산한다.
            let (desired_width, desired_height) = crate::dialogs::helpers::design_to_window_size(
                self.hwnd,
                Self::WIDTH,
                target_height,
            );
            let win_width = desired_width.min(work_width);
            let win_height = desired_height.min(work_height);
            let mut rect = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut rect);
            let x = if win_width >= work_width {
                work.left
            } else {
                rect.left.clamp(work.left, work.right - win_width)
            };
            let y = if win_height >= work_height {
                work.top
            } else {
                rect.top.clamp(work.top, work.bottom - win_height)
            };
            // SWP_NOCOPYBITS: 크기가 바뀔 때 이전 탭의 픽셀이 새 위치로 복사되어
            // 잔상으로 남는 것을 막는다.
            let _ = SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                x,
                y,
                win_width,
                win_height,
                SWP_NOZORDER | SWP_FRAMECHANGED | SWP_NOCOPYBITS,
            );
        }
        self.layout_for_current_size();
        self.redraw_all();
    }

    /// 탭 전환·크기 조정 후 부모와 모든 자식을 다시 그리도록 예약한다.
    ///
    /// 이 함수는 dialog state를 가변 대여한 상태에서 호출된다. `RDW_UPDATENOW`로
    /// 동기 paint를 강제하면 owner-draw 버튼의 `WM_DRAWITEM`이 재진입하고,
    /// `DialogHost`가 같은 state를 다시 대여하지 못해 색상 버튼이 빈 채로 남는다.
    pub(super) fn redraw_all(&self) {
        // SAFETY: self.hwnd는 설정창 수명 동안 유효한 핸들이다.
        unsafe {
            let _ = RedrawWindow(
                self.hwnd,
                std::ptr::null(),
                std::ptr::null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
            );
        }
    }

    /// 사용자가 테두리를 끌어 바꾼 client 크기에 tab, 하단 button, scrollbar를 맞춘다.
    pub(super) fn layout_for_current_size(&mut self) {
        // SAFETY: self.hwnd와 자식 컨트롤은 설정창 수명 동안 유효하다.
        unsafe {
            let mut client = RECT::default();
            if GetClientRect(self.hwnd, &mut client) == 0 {
                return;
            }
            let height = (client.bottom - client.top).max(1);
            let dpi = crate::dpi::dpi_for_window(self.hwnd);
            let s = |v: i32| crate::dpi::scale(v, dpi);
            let content_height = s(self.target_height_for_tab(self.current_tab));
            let scroll_max = (content_height - height).max(0);
            let new_scroll_pos = self.scroll_pos.clamp(0, scroll_max);

            // 수동으로 창을 낮춘 경우에도 표준 세로 scrollbar를 즉시 표시한다.
            let _ = ShowScrollBar(self.hwnd, SB_VERT, if scroll_max > 0 { 1 } else { 0 });
            let _ = GetClientRect(self.hwnd, &mut client);
            let width = (client.right - client.left).max(1);
            let height = (client.bottom - client.top).max(1);

            if new_scroll_pos != self.scroll_pos {
                let delta = self.scroll_pos - new_scroll_pos;
                self.offset_scroll_children(delta);
            }
            self.scroll_pos = new_scroll_pos;
            self.scroll_max = scroll_max;

            // Tab은 가로로 창을 채우고, 세로로는 기존 내용 또는 viewport 중 큰 쪽을 쓴다.
            let tab_hwnd = GetDlgItem(self.hwnd, ctrl_id::TAB_CONTROL as i32);
            if !tab_hwnd.is_null() {
                let _ = SetWindowPos(
                    tab_hwnd,
                    std::ptr::null_mut(),
                    s(5),
                    s(5) - new_scroll_pos,
                    (width - s(10)).max(1),
                    (content_height.max(height) - s(70)).max(1),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }

            // 내용이 모두 보이면 하단에 고정하고, 스크롤 중이면 기존처럼 내용 끝에 둔다.
            let button_y = if scroll_max == 0 {
                height - s(65)
            } else {
                content_height - s(65) - new_scroll_pos
            };
            let mut button_right = width - s(15);
            for id in [ctrl_id::CLOSE, ctrl_id::APPLY] {
                let button = GetDlgItem(self.hwnd, id as i32);
                if !button.is_null() {
                    let mut button_rect = RECT::default();
                    if GetWindowRect(button, &mut button_rect) == 0 {
                        continue;
                    }
                    let button_width = button_rect.right - button_rect.left;
                    let button_height = button_rect.bottom - button_rect.top;
                    button_right -= button_width;
                    let _ = SetWindowPos(
                        button,
                        std::ptr::null_mut(),
                        button_right,
                        button_y,
                        button_width,
                        button_height,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    button_right -= s(5);
                }
            }

            let scroll_info = SCROLLINFO {
                cbSize: size_of::<SCROLLINFO>() as u32,
                fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                nMin: 0,
                nMax: content_height.saturating_sub(1),
                nPage: height as u32,
                nPos: new_scroll_pos,
                ..Default::default()
            };
            SetScrollInfo(self.hwnd, SB_VERT, &scroll_info, 1);
        }
    }

    /// 탭 전환 직전에 스크롤 오프셋을 맨 위로 되돌린다.
    ///
    /// `scroll_to(0)`과 달리 `scroll_max`(아직 이전 탭 값)를 참조하지 않으므로,
    /// 새 탭의 높이를 계산하기 전에도 자식 위치를 확실히 원점으로 맞춘다.
    pub(super) fn reset_scroll_offset(&mut self) {
        if self.scroll_pos == 0 {
            return;
        }
        // SAFETY: self.hwnd와 그 자식 컨트롤은 설정창 수명 동안 유효하다.
        unsafe {
            self.offset_scroll_children(self.scroll_pos);
        }
        self.scroll_pos = 0;
    }

    /// 낮은 해상도에서 잘린 설정 내용을 세로로 이동한다.
    pub(super) fn scroll_to(&mut self, position: i32) {
        let new_pos = position.clamp(0, self.scroll_max);
        if new_pos == self.scroll_pos {
            return;
        }

        // SAFETY: self.hwnd와 그 자식 컨트롤은 설정창 수명 동안 유효하다.
        unsafe {
            let delta = self.scroll_pos - new_pos;
            self.offset_scroll_children(delta);
            self.scroll_pos = new_pos;
            let scroll_info = SCROLLINFO {
                cbSize: size_of::<SCROLLINFO>() as u32,
                fMask: SIF_POS,
                nPos: new_pos,
                ..Default::default()
            };
            SetScrollInfo(self.hwnd, SB_VERT, &scroll_info, 1);
            let _ = InvalidateRect(self.hwnd, std::ptr::null(), 1);
            let _ = UpdateWindow(self.hwnd);
        }
    }

    /// 화면 밖의 항목도 빠지지 않도록 모든 직접 자식을 이동한다.
    pub(super) unsafe fn offset_scroll_children(&self, delta: i32) {
        unsafe fn offset(parent: HWND, child: HWND, delta: i32) {
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(child, &mut rect) } == 0 {
                return;
            }
            let mut top_left = POINT {
                x: rect.left,
                y: rect.top,
            };
            unsafe {
                let _ = ScreenToClient(parent, &mut top_left);
                let _ = SetWindowPos(
                    child,
                    std::ptr::null_mut(),
                    top_left.x,
                    top_left.y + delta,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }

        // 등록 목록(tab_controls/engine_controls)을 순회하면 양쪽에 중복 등록된
        // 컨트롤이 두 번 이동하고, 어느 쪽에도 없는 컨트롤(예: EzTrans 경고 라벨)은
        // 아예 이동하지 않는다. 실제 자식 창을 열거해 하나씩만 정확히 옮긴다.
        unsafe {
            let mut children: Vec<HWND> = Vec::new();
            let _ = EnumChildWindows(
                self.hwnd,
                Some(collect_direct_child),
                &mut children as *mut Vec<HWND> as isize,
            );
            for child in children {
                // 탭 컨트롤의 손자(자식의 자식)는 부모를 따라 함께 움직인다.
                if GetParent(child) == self.hwnd {
                    offset(self.hwnd, child, delta);
                }
            }
        }
    }

    pub(super) fn handle_vertical_scroll(&mut self, wparam: WPARAM) {
        if self.scroll_max == 0 {
            return;
        }

        let command = (wparam & 0xffff) as i32;
        let dpi = crate::dpi::dpi_for_window(self.hwnd);
        let line = crate::dpi::scale(24, dpi);
        let mut client = RECT::default();
        // SAFETY: self.hwnd는 유효한 설정창 핸들이다.
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
        }
        let page = (client.bottom - client.top - line).max(line);
        let target = match command {
            value if value == SB_LINEUP => self.scroll_pos - line,
            value if value == SB_LINEDOWN => self.scroll_pos + line,
            value if value == SB_PAGEUP => self.scroll_pos - page,
            value if value == SB_PAGEDOWN => self.scroll_pos + page,
            value if value == SB_TOP => 0,
            value if value == SB_BOTTOM => self.scroll_max,
            value if value == SB_THUMBTRACK || value == SB_THUMBPOSITION => {
                let mut info = SCROLLINFO {
                    cbSize: size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_TRACKPOS,
                    ..Default::default()
                };
                // SAFETY: info는 쓰기 가능한 SCROLLINFO이고 hwnd에 SB_VERT가 있다.
                if unsafe { GetScrollInfo(self.hwnd, SB_VERT, &mut info) } != 0 {
                    info.nTrackPos
                } else {
                    ((wparam >> 16) & 0xffff) as i32
                }
            }
            _ => return,
        };
        self.scroll_to(target);
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/settings/layout.rs"]
mod tests;
