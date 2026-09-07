//! 컨트롤 초기화, 명령 처리, 폰트/저장 대화상자, 크기 재배치.

use std::thread;
use windows_core::{Error, HRESULT};
use windows_sys::Win32::{Foundation::*, UI::Controls::*, UI::WindowsAndMessaging::*};

use super::{BacklogDialog, NEXT_SAVE_TOKEN, Result, SaveResult, WM_BACKLOG_SAVE_RESULT, ctrl_id};
use crate::app::backlog::{BacklogFilter, MAX_BACKLOG_ENTRIES, MAX_BACKLOG_TEXT_BYTES};
use crate::dialogs::file_dialog::{FileFilter, save_file};
use crate::dialogs::font::{FontDialog, FontDialogConfig, FontStyle};
use crate::win32::to_wide;

// RichEdit messages not exported by windows-sys' Controls module.
const EM_EXLIMITTEXT: u32 = 0x0435;
const EM_SETBKGNDCOLOR: u32 = 0x0443;

impl BacklogDialog {
    pub(super) fn initialize_controls(&mut self) -> Result<()> {
        let get_control = |id| {
            let control = unsafe { GetDlgItem(self.hwnd, id) };
            if control.is_null() {
                Err(Error::new(
                    HRESULT(E_FAIL),
                    format!("백로그 컨트롤 ID {id}를 찾을 수 없습니다"),
                ))
            } else {
                Ok(control)
            }
        };
        self.richedit = get_control(ctrl_id::RICHEDIT as i32)?;
        self.group_filter = get_control(ctrl_id::GROUP_FILTER as i32)?;
        self.group_action = get_control(ctrl_id::GROUP_ACTION as i32)?;
        for id in [
            ctrl_id::CHK_LINEFEED,
            ctrl_id::RADIO_ORIGINAL,
            ctrl_id::RADIO_TRANSLATION,
            ctrl_id::RADIO_ALL,
            ctrl_id::BTN_CLEAR,
            ctrl_id::BTN_SAVE,
            ctrl_id::BTN_FONT,
        ] {
            get_control(id as i32)?;
        }
        unsafe {
            let _ = CheckDlgButton(self.hwnd, ctrl_id::RADIO_ALL as i32, BST_CHECKED);
            let _ = CheckDlgButton(self.hwnd, ctrl_id::CHK_LINEFEED as i32, BST_CHECKED);
            let _ = SendMessageW(self.richedit, EM_SETBKGNDCOLOR, 1, 0);
            // Store 상한에 포맷 label/newline 여유를 더한 명시적 문자 제한.
            let display_limit =
                MAX_BACKLOG_TEXT_BYTES.saturating_add(MAX_BACKLOG_ENTRIES.saturating_mul(128));
            let _ = SendMessageW(self.richedit, EM_EXLIMITTEXT, 0, display_limit as isize);
        }
        self.refresh_richedit();
        Ok(())
    }

    pub(super) fn handle_command(&mut self, cmd: u16) {
        use ctrl_id::*;
        match cmd {
            CHK_LINEFEED => {
                self.add_linefeed = !self.add_linefeed;
                self.refresh_richedit();
            }
            RADIO_ORIGINAL => {
                self.filter = BacklogFilter::Original;
                self.refresh_richedit();
            }
            RADIO_TRANSLATION => {
                self.filter = BacklogFilter::Translation;
                self.refresh_richedit();
            }
            RADIO_ALL => {
                self.filter = BacklogFilter::All;
                self.refresh_richedit();
            }
            BTN_CLEAR => self.clear_richedit(),
            BTN_SAVE => self.save_to_file(),
            BTN_FONT => self.choose_font(),
            _ => {}
        }
        self.refresh_if_dirty();
    }

    /// 폰트 선택 대화상자
    fn choose_font(&mut self) {
        let cfg = FontDialogConfig {
            initial_face: self.font_face.clone(),
            initial_style: FontStyle {
                bold: false,
                italic: self.font_italic,
            },
            initial_point_size: self.font_point_size,
            no_activate: false,
        };

        let Some(result) = FontDialog::show(self.hwnd, cfg) else {
            return;
        };

        self.font_face = Some(result.face_name);
        self.font_italic = result.style.italic;
        if result.point_size > 0 {
            self.font_point_size = result.point_size;
        }
        self.refresh_richedit();
    }

    /// 파일로 저장
    fn save_to_file(&mut self) {
        if self.save_in_progress {
            return;
        }
        let filters = [
            FileFilter {
                name: "텍스트 파일 (*.txt)",
                spec: "*.txt",
            },
            FileFilter {
                name: "모든 파일 (*.*)",
                spec: "*.*",
            },
        ];
        let path = match save_file(self.hwnd, "백로그 저장", &filters, Some("txt"), None) {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                tracing::error!("백로그 저장 대화상자 오류: {error}");
                let message = to_wide(&format!("저장 대화상자를 열 수 없습니다.\n{error}"));
                unsafe {
                    let _ = MessageBoxW(
                        self.hwnd,
                        message.as_ptr(),
                        crate::win32::to_wide("오류").as_ptr(),
                        MB_ICONERROR,
                    );
                }
                return;
            }
        };

        let store = self.store.clone();
        let result_slot = std::sync::Arc::clone(&self.save_result);
        let hwnd = self.hwnd as usize;
        let token = NEXT_SAVE_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.save_token = token;
        self.save_in_progress = true;
        self.save_worker = thread::Builder::new()
            .name("anemone-backlog-save".into())
            .spawn(move || {
                let result = store.export_utf8(&path);
                *result_slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(SaveResult { result });
                // SAFETY: hwnd is the live backlog dialog while the worker result is pending.
                if unsafe { PostMessageW(hwnd as HWND, WM_BACKLOG_SAVE_RESULT, token as usize, 0) }
                    == 0
                {
                    tracing::debug!("백로그 저장 결과 알림 실패");
                }
            })
            .ok();
        if self.save_worker.is_none() {
            self.save_in_progress = false;
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "백로그 저장 오류",
                "저장 워커를 시작하지 못했습니다.",
            );
        }
    }

    /// 윈도우 크기 변경 시 컨트롤 재배치
    ///
    /// RichEdit는 새 client size에 맞춰 늘리고 하단 그룹은 아래쪽에 고정한다.
    pub(super) fn on_size(&self, width: i32, height: i32) {
        unsafe {
            let dpi = crate::dpi::dpi_for_window(self.hwnd);
            let s = |v: i32| crate::dpi::scale(v, dpi);
            let margin = s(10);
            let group_y = height - s(100);
            let filter_width = (width - s(240)).max(s(250));
            let action_x = width - s(220);

            let _ = SetWindowPos(
                self.richedit,
                std::ptr::null_mut(),
                margin,
                margin,
                (width - margin * 2).max(1),
                (height - s(120)).max(1),
                SWP_NOZORDER,
            );

            let move_to = |ctrl: HWND, x: i32, y: i32, w: i32, h: i32| {
                let _ = SetWindowPos(ctrl, std::ptr::null_mut(), x, y, w, h, SWP_NOZORDER);
            };
            let move_ctrl = |id: u16, x: i32, y: i32, w: i32, h: i32| {
                let ctrl = GetDlgItem(self.hwnd, id as i32);
                if !ctrl.is_null() {
                    move_to(ctrl, x, y, w, h);
                }
            };

            move_to(self.group_filter, margin, group_y, filter_width, s(60));
            move_to(self.group_action, action_x, group_y, s(210), s(60));

            use ctrl_id::*;
            let option_y = group_y + s(20);
            move_ctrl(RADIO_ORIGINAL, margin + s(10), option_y, s(80), s(20));
            move_ctrl(RADIO_TRANSLATION, margin + s(95), option_y, s(80), s(20));
            move_ctrl(RADIO_ALL, margin + s(180), option_y, s(60), s(20));
            move_ctrl(
                CHK_LINEFEED,
                margin + filter_width - s(100),
                option_y,
                s(90),
                s(20),
            );
            let button_y = group_y + s(22);
            move_ctrl(BTN_CLEAR, action_x + s(10), button_y, s(55), s(28));
            move_ctrl(BTN_SAVE, action_x + s(75), button_y, s(55), s(28));
            move_ctrl(BTN_FONT, action_x + s(140), button_y, s(55), s(28));
        }
    }
}
