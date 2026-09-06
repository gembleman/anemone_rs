//! Owner-draw 색상 버튼: 현재 설정값을 swatch + 텍스트로 그린다.

use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, UI::Controls::*, UI::WindowsAndMessaging::*,
};

use super::{SettingsDialog, ctrl_id};

impl SettingsDialog {
    /// ctrl_id로부터 미리보기 색상(ARGB)을 찾는다
    pub(super) fn color_for_button(&self, id: u16) -> Option<u32> {
        use crate::config::{ColorType, TextType};
        let cfg = self.draft.borrow();
        let argb = match id {
            ctrl_id::BACKGROUND_COLOR => cfg.background_color,
            ctrl_id::BORDER_COLOR => cfg.border_color,
            ctrl_id::NAME_COLOR => cfg.get_text_color(TextType::Name, ColorType::Primary),
            ctrl_id::NAME_OUTLINE1 => cfg.get_text_color(TextType::Name, ColorType::Outline1),
            ctrl_id::NAME_OUTLINE2 => cfg.get_text_color(TextType::Name, ColorType::Outline2),
            ctrl_id::NAME_SHADOW_COLOR => cfg.get_text_color(TextType::Name, ColorType::Shadow),
            ctrl_id::ORG_COLOR => cfg.get_text_color(TextType::Original, ColorType::Primary),
            ctrl_id::ORG_OUTLINE1 => cfg.get_text_color(TextType::Original, ColorType::Outline1),
            ctrl_id::ORG_OUTLINE2 => cfg.get_text_color(TextType::Original, ColorType::Outline2),
            ctrl_id::ORG_SHADOW_COLOR => cfg.get_text_color(TextType::Original, ColorType::Shadow),
            ctrl_id::TRANS_COLOR => cfg.get_text_color(TextType::Translation, ColorType::Primary),
            ctrl_id::TRANS_OUTLINE1 => {
                cfg.get_text_color(TextType::Translation, ColorType::Outline1)
            }
            ctrl_id::TRANS_OUTLINE2 => {
                cfg.get_text_color(TextType::Translation, ColorType::Outline2)
            }
            ctrl_id::TRANS_SHADOW_COLOR => {
                cfg.get_text_color(TextType::Translation, ColorType::Shadow)
            }
            _ => return None,
        };
        Some(argb)
    }

    /// 색상 버튼에 swatch와 text를 그린다.
    pub(super) fn draw_color_button(&self, dis: &DRAWITEMSTRUCT) {
        let id = dis.CtlID as u16;
        let Some(argb) = self.color_for_button(id) else {
            return;
        };
        // SAFETY: hDC and rcItem are valid for the duration of WM_DRAWITEM.
        unsafe {
            let hdc = dis.hDC;
            let rc = dis.rcItem;

            // 배경: 시스템 버튼 면색
            let bg_brush = GetSysColorBrush(COLOR_BTNFACE);
            let _ = FillRect(hdc, &rc, bg_brush);

            // 테두리 (눌림 / 포커스 상태에 따라 다르게)
            let pressed = (dis.itemState & ODS_SELECTED) != 0;
            if pressed {
                let _ = DrawEdge(hdc, &mut { rc }, EDGE_SUNKEN, BF_RECT);
            } else {
                let _ = DrawEdge(hdc, &mut { rc }, EDGE_RAISED, BF_RECT);
            }

            // 색상 스왓치 (왼쪽 1/3 영역)
            let pad = 4;
            let swatch_w = ((rc.right - rc.left - pad * 3) / 3).max(12);
            let swatch_rc = RECT {
                left: rc.left + pad,
                top: rc.top + pad,
                right: rc.left + pad + swatch_w,
                bottom: rc.bottom - pad,
            };
            // ARGB → COLORREF (0x00BBGGRR)
            let r = (argb >> 16) & 0xFF;
            let g = (argb >> 8) & 0xFF;
            let b = argb & 0xFF;
            let colorref = (b << 16) | (g << 8) | r;
            let brush = CreateSolidBrush(colorref);
            let _ = FillRect(hdc, &swatch_rc, brush);
            // 스왓치 외곽선
            let pen_color = 0x00808080;
            let pen = CreatePen(PS_SOLID, 1, pen_color);
            let old_pen = SelectObject(hdc, pen);
            let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
            let _ = Rectangle(
                hdc,
                swatch_rc.left,
                swatch_rc.top,
                swatch_rc.right,
                swatch_rc.bottom,
            );
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_brush);
            let _ = DeleteObject(pen);
            let _ = DeleteObject(brush);

            // 텍스트 (오른쪽 2/3 영역)
            let mut text_rc = RECT {
                left: swatch_rc.right + pad,
                top: rc.top,
                right: rc.right - pad,
                bottom: rc.bottom,
            };
            let mut buf = [0u16; 64];
            let len = GetWindowTextW(dis.hwndItem, buf.as_mut_ptr(), buf.len() as i32);
            if len > 0 {
                let _ = SetBkMode(hdc, TRANSPARENT as i32);
                let _ = DrawTextW(
                    hdc,
                    buf.as_ptr(),
                    len,
                    &mut text_rc,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
            }

            // 포커스 사각형
            if (dis.itemState & ODS_FOCUS) != 0 {
                let _ = DrawFocusRect(hdc, &rc);
            }
        }
    }

    /// 색상 변경 후 해당 버튼만 강제 다시 그리기
    pub(super) fn invalidate_color_button(&self, id: u16) {
        Self::invalidate_color_button_for(self.hwnd, id);
    }

    pub(super) fn invalidate_color_button_for(hwnd: HWND, id: u16) {
        // SAFETY: hwnd is a valid settings window; GetDlgItem returns its child control.
        unsafe {
            let h = GetDlgItem(hwnd, id as i32);
            if !h.is_null() {
                let _ = InvalidateRect(h, std::ptr::null(), 1);
            }
        }
    }
}
