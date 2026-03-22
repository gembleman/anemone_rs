//! 설정 대화상자 명령/이벤트 핸들러

use windows::{
    Win32::{
        Foundation::*, UI::Controls::*, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::ctrl_id;
use super::SettingsDialog;
use crate::constants::{CB_GETCURSEL, CBN_SELCHANGE};
use crate::config::{ColorType, TextAlign, TextType};
use crate::util::to_wide;
use crate::dialogs::color::ColorDialog;
use crate::dialogs::font::{FontDialog, FontDialogConfig, FontStyle};

impl SettingsDialog {
    /// 명령 처리
    pub(super) fn handle_command(&mut self, cmd: u16, notify_code: u32) {
        // ComboBox 선택 변경은 별도 처리
        if notify_code == CBN_SELCHANGE {
            self.handle_combobox(cmd);
            return;
        }

        use ctrl_id::*;

        match cmd {
            // SAFETY: self.hwnd is a valid window handle from dialog creation.
            CLOSE => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },

            // 배경 색상
            BACKGROUND_COLOR => {
                let initial = self.config.borrow().background_color;
                if let Some(result) = ColorDialog::show_simple(self.hwnd, initial) {
                    self.config.borrow_mut().background_color = result.argb;
                    self.notify_change();
                }
            }

            // 배경 표시 토글
            BACKGROUND_SWITCH => {
                self.config.borrow_mut().toggle_background_visible();
                self.notify_change();
            }

            // NAME 색상들
            NAME_COLOR => self.handle_color_button(TextType::Name, ColorType::Primary),
            NAME_OUTLINE1 => self.handle_color_button(TextType::Name, ColorType::Outline1),
            NAME_OUTLINE2 => self.handle_color_button(TextType::Name, ColorType::Outline2),
            NAME_SHADOW_COLOR => self.handle_color_button(TextType::Name, ColorType::Shadow),
            NAME_FONT => self.handle_font_button(TextType::Name),
            NAME_SHADOW => {
                self.config.borrow_mut().toggle_shadow(TextType::Name);
                self.notify_change();
            }

            // ORG 색상들
            ORG_COLOR => self.handle_color_button(TextType::Original, ColorType::Primary),
            ORG_OUTLINE1 => self.handle_color_button(TextType::Original, ColorType::Outline1),
            ORG_OUTLINE2 => self.handle_color_button(TextType::Original, ColorType::Outline2),
            ORG_SHADOW_COLOR => self.handle_color_button(TextType::Original, ColorType::Shadow),
            ORG_FONT => self.handle_font_button(TextType::Original),
            ORG_SHADOW => {
                self.config.borrow_mut().toggle_shadow(TextType::Original);
                self.notify_change();
            }

            // TRANS 색상들
            TRANS_COLOR => self.handle_color_button(TextType::Translation, ColorType::Primary),
            TRANS_OUTLINE1 => self.handle_color_button(TextType::Translation, ColorType::Outline1),
            TRANS_OUTLINE2 => self.handle_color_button(TextType::Translation, ColorType::Outline2),
            TRANS_SHADOW_COLOR => {
                self.handle_color_button(TextType::Translation, ColorType::Shadow)
            }
            TRANS_FONT => self.handle_font_button(TextType::Translation),
            TRANS_SHADOW => {
                self.config
                    .borrow_mut()
                    .toggle_shadow(TextType::Translation);
                self.notify_change();
            }

            // 테두리 설정
            BORDER_MODE => {
                self.config.borrow_mut().toggle_border_visible();
                self.notify_change();
            }
            BORDER_COLOR => {
                let initial = self.config.borrow().border_color;
                if let Some(result) = ColorDialog::show_simple(self.hwnd, initial) {
                    self.config.borrow_mut().border_color = result.argb;
                    self.notify_change();
                }
            }

            // 표시 옵션
            PRINT_ORGTEXT => {
                let mut cfg = self.config.borrow_mut();
                cfg.show_original = !cfg.show_original;
                drop(cfg);
                self.notify_change();
            }
            PRINT_TRANSTEXT => {
                let mut cfg = self.config.borrow_mut();
                cfg.show_translation = !cfg.show_translation;
                drop(cfg);
                self.notify_change();
            }
            PRINT_ORGNAME => {
                let mut cfg = self.config.borrow_mut();
                cfg.show_name = !cfg.show_name;
                drop(cfg);
                self.notify_change();
            }
            SEPERATE_NAME => {
                let mut cfg = self.config.borrow_mut();
                cfg.separate_name = !cfg.separate_name;
                drop(cfg);
                self.notify_change();
            }
            REPEAT_TEXT => {
                // 모드 순환 (0→1→2→3→4→0)
                let mut cfg = self.config.borrow_mut();
                cfg.repeat_text_mode = (cfg.repeat_text_mode + 1) % 5;
                let new_mode = cfg.repeat_text_mode;
                drop(cfg);
                // 버튼 텍스트 업데이트
                // SAFETY: self.hwnd is valid; GetDlgItem returns a valid control handle.
                unsafe {
                    if let Ok(btn) = GetDlgItem(Some(self.hwnd), REPEAT_TEXT as i32) {
                        if !btn.is_invalid() {
                            let text = format!("반복:{}", new_mode);
                            let text_wide = to_wide(&text);
                            let _ = SetWindowTextW(btn, PCWSTR(text_wide.as_ptr()));
                        }
                    }
                }
                self.notify_change();
            }

            // 텍스트 정렬
            TEXTALIGN_LEFT => {
                self.config.borrow_mut().text_align = TextAlign::Left;
                self.notify_change();
            }
            TEXTALIGN_MID => {
                self.config.borrow_mut().text_align = TextAlign::Center;
                self.notify_change();
            }
            TEXTALIGN_RIGHT => {
                self.config.borrow_mut().text_align = TextAlign::Right;
                self.notify_change();
            }

            // 윈도우 옵션
            TOPMOST => {
                let mut cfg = self.config.borrow_mut();
                cfg.window_topmost = !cfg.window_topmost;
                drop(cfg);
                self.notify_change();
            }
            USE_MAGNETIC => {
                self.config.borrow_mut().toggle_magnetic_mode();
                self.notify_change();
            }
            MAGNETIC_MINIMIZE => {
                let mut cfg = self.config.borrow_mut();
                cfg.magnetic_minimize = !cfg.magnetic_minimize;
                drop(cfg);
                self.notify_change();
            }
            HIDEWIN => {
                let mut cfg = self.config.borrow_mut();
                cfg.temp_window_hide = !cfg.temp_window_hide;
                drop(cfg);
                self.notify_change();
            }
            CLIPBOARD_WATCH => {
                self.config.borrow_mut().toggle_clipboard_watch();
                self.notify_change();
            }
            WNDCLICK_THROUGH => {
                self.config.borrow_mut().toggle_click_through();
                self.notify_change();
            }

            // 텍스트 크기 +/-
            TEXTSIZE_MINUS => {
                let new_size = {
                    let mut cfg = self.config.borrow_mut();
                    let current = cfg.translation_style.size;
                    if current > 6 {
                        cfg.set_all_text_size(ColorType::Primary, current - 1);
                        current - 1
                    } else {
                        current
                    }
                };
                self.update_textsize_ui(new_size);
                self.notify_change();
            }
            TEXTSIZE_PLUS => {
                let new_size = {
                    let mut cfg = self.config.borrow_mut();
                    let current = cfg.translation_style.size;
                    if current < 100 {
                        cfg.set_all_text_size(ColorType::Primary, current + 1);
                        current + 1
                    } else {
                        current
                    }
                };
                self.update_textsize_ui(new_size);
                self.notify_change();
            }

            // 외곽선1 +/-
            OUTLINE1_MINUS => {
                let mut cfg = self.config.borrow_mut();
                let current = cfg.translation_style.outline1_size;
                if current > 0 {
                    cfg.set_all_text_size(ColorType::Outline1, current - 1);
                }
                drop(cfg);
                self.notify_change();
            }
            OUTLINE1_PLUS => {
                let mut cfg = self.config.borrow_mut();
                let current = cfg.translation_style.outline1_size;
                if current < 20 {
                    cfg.set_all_text_size(ColorType::Outline1, current + 1);
                }
                drop(cfg);
                self.notify_change();
            }

            // 외곽선2 +/-
            OUTLINE2_MINUS => {
                let mut cfg = self.config.borrow_mut();
                let current = cfg.translation_style.outline2_size;
                if current > 0 {
                    cfg.set_all_text_size(ColorType::Outline2, current - 1);
                }
                drop(cfg);
                self.notify_change();
            }
            OUTLINE2_PLUS => {
                let mut cfg = self.config.borrow_mut();
                let current = cfg.translation_style.outline2_size;
                if current < 20 {
                    cfg.set_all_text_size(ColorType::Outline2, current + 1);
                }
                drop(cfg);
                self.notify_change();
            }

            // 자동 언어 감지 체크박스
            TRANS_AUTO_DETECT => {
                let mut cfg = self.config.borrow_mut();
                cfg.translation.auto_detect = !cfg.translation.auto_detect;
                drop(cfg);
                self.notify_change();
            }

            // EzTrans DLL 찾아보기
            EZTRANS_DLL_BROWSE => {
                if let Some(path) = self.browse_dll_file("J2KEngine.dll 선택") {
                    self.config.borrow_mut().translation.eztrans_dll_path = path.clone();
                    // SAFETY: self.hwnd is valid; GetDlgItem returns a valid edit control.
                    unsafe {
                        if let Ok(edit) = GetDlgItem(Some(self.hwnd), EZTRANS_DLL_EDIT as i32) {
                            if !edit.is_invalid() {
                                let text_wide = to_wide(&path);
                                let _ = SetWindowTextW(edit, PCWSTR(text_wide.as_ptr()));
                            }
                        }
                    }
                    self.sync_translation_manager();
                    self.notify_change();
                }
            }

            // EzTrans Dat 폴더 찾아보기
            EZTRANS_DAT_BROWSE => {
                if let Some(path) = self.browse_folder_with_title("EzTrans Dat 폴더 선택") {
                    self.config.borrow_mut().translation.eztrans_dat_path = path.clone();
                    // SAFETY: self.hwnd is valid; GetDlgItem returns a valid edit control.
                    unsafe {
                        if let Ok(edit) = GetDlgItem(Some(self.hwnd), EZTRANS_DAT_EDIT as i32) {
                            if !edit.is_invalid() {
                                let text_wide = to_wide(&path);
                                let _ = SetWindowTextW(edit, PCWSTR(text_wide.as_ptr()));
                            }
                        }
                    }
                    self.sync_translation_manager();
                    self.notify_change();
                }
            }

            _ => {}
        }
    }

    /// 색상 버튼 처리
    fn handle_color_button(&mut self, text_type: TextType, color_type: ColorType) {
        let initial = self.config.borrow().get_text_color(text_type, color_type);
        if let Some(result) = ColorDialog::show_simple(self.hwnd, initial) {
            self.config
                .borrow_mut()
                .set_text_color(text_type, color_type, result.argb);
            self.notify_change();
        }
    }

    /// 폰트 버튼 처리
    fn handle_font_button(&mut self, text_type: TextType) {
        let (face, style_bits, size) = {
            let cfg = self.config.borrow();
            let style = cfg.get_text_style(text_type);
            (style.font_face.clone(), style.font_style, style.size)
        };

        let font_config = FontDialogConfig {
            initial_face: Some(face),
            initial_style: FontStyle::from_bits(style_bits),
            initial_point_size: size,
            no_activate: true,
        };

        if let Some(result) = FontDialog::show(self.hwnd, font_config) {
            let mut cfg = self.config.borrow_mut();
            let style = cfg.get_text_style_mut(text_type);
            style.font_face = result.face_name;
            style.font_style = result.style.to_bits();
            drop(cfg);
            self.notify_change();
        }
    }

    /// 트랙바 변경 처리
    pub(super) fn handle_trackbar(&mut self, id: u16, value: i32) {
        use ctrl_id::*;

        match id {
            BACKGROUND_TRACKBAR => {
                let mut cfg = self.config.borrow_mut();
                let rgb = cfg.background_color & 0x00FFFFFF;
                cfg.background_color = ((value as u32) << 24) | rgb;
                drop(cfg);
                self.notify_change();
            }
            TEXTSIZE_TRACKBAR => {
                self.config
                    .borrow_mut()
                    .set_all_text_size(ColorType::Primary, value);
                // 크기 레이블 업데이트
                // SAFETY: self.hwnd is valid; GetDlgItem returns a valid label control.
                unsafe {
                    if let Ok(label) = GetDlgItem(Some(self.hwnd), TEXTSIZE_TEXT as i32) {
                        if !label.is_invalid() {
                            let text = format!("크기: {}", value);
                            let text_wide = to_wide(&text);
                            let _ = SetWindowTextW(label, PCWSTR(text_wide.as_ptr()));
                        }
                    }
                }
                self.notify_change();
            }
            OUTLINE1_TRACKBAR => {
                self.config
                    .borrow_mut()
                    .set_all_text_size(ColorType::Outline1, value);
                self.notify_change();
            }
            OUTLINE2_TRACKBAR => {
                self.config
                    .borrow_mut()
                    .set_all_text_size(ColorType::Outline2, value);
                self.notify_change();
            }
            SHADOW_X_TRACKBAR => {
                self.config.borrow_mut().shadow_offset_x = value;
                self.notify_change();
            }
            SHADOW_Y_TRACKBAR => {
                self.config.borrow_mut().shadow_offset_y = value;
                self.notify_change();
            }
            MARGIN_X_TRACKBAR => {
                self.config.borrow_mut().text_margin_x = value;
                self.notify_change();
            }
            MARGIN_Y_TRACKBAR => {
                self.config.borrow_mut().text_margin_y = value;
                self.notify_change();
            }
            MARGIN_NAME_TRACKBAR => {
                self.config.borrow_mut().name_margin = value;
                self.notify_change();
            }
            BORDER_SIZE_TRACKBAR => {
                self.config.borrow_mut().border_width = value;
                self.notify_change();
            }
            _ => {}
        }
    }

    /// ComboBox 선택 변경 처리
    pub(super) fn handle_combobox(&mut self, id: u16) {
        use ctrl_id::*;

        // SAFETY: self.hwnd is valid; GetDlgItem and SendMessageW use valid handles.
        unsafe {
            let combo = match GetDlgItem(Some(self.hwnd), id as i32) {
                Ok(h) if !h.is_invalid() => h,
                _ => return,
            };
            let sel =
                SendMessageW(combo, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as usize;

            match id {
                TRANS_ENGINE => {
                    use crate::translation::TranslationEngine;
                    let engine = TranslationEngine::from_u8(sel as u8);
                    self.config.borrow_mut().translation.set_engine(engine);
                    self.sync_translation_manager();
                    self.notify_change();
                }
                TRANS_SOURCE_LANG => {
                    let engine = self.config.borrow().translation.get_engine();
                    self.config
                        .borrow_mut()
                        .translation
                        .set_source_lang_by_index(sel, engine);
                    self.sync_translation_manager();
                    self.notify_change();
                }
                TRANS_TARGET_LANG => {
                    let engine = self.config.borrow().translation.get_engine();
                    self.config
                        .borrow_mut()
                        .translation
                        .set_target_lang_by_index(sel, engine);
                    self.sync_translation_manager();
                    self.notify_change();
                }
                _ => {}
            }
        }
    }

    /// 번역 매니저 설정 동기화
    fn sync_translation_manager(&self) {
        use crate::translation::get_translation_manager;
        let config = self.config.borrow();
        let manager = get_translation_manager();
        if let Ok(mut mgr) = manager.lock() {
            mgr.set_engine(config.translation.get_engine());
            mgr.set_source_language(config.translation.get_source_language());
            mgr.set_target_language(config.translation.get_target_language());

            // EzTrans 초기화 (경로가 설정되어 있을 경우)
            if !config.translation.eztrans_dll_path.is_empty()
                && !config.translation.eztrans_dat_path.is_empty()
            {
                if let Err(e) = mgr.init_eztrans(
                    &config.translation.eztrans_dll_path,
                    &config.translation.eztrans_dat_path,
                ) {
                    tracing::warn!("EzTrans init failed in sync: {e}");
                }
            }

            // DeepL API 키 설정
            if !config.translation.deepl_api_key.is_empty() {
                mgr.set_deepl_api_key(config.translation.deepl_api_key.clone());
            }
        }
    }

    /// 제목 지정 폴더 브라우저 열기
    fn browse_folder_with_title(&self, title: &str) -> Option<String> {
        use windows::Win32::System::Com::{
            CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
        };
        use windows::Win32::UI::Shell::{
            FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem, SIGDN_FILESYSPATH,
        };

        // SAFETY: COM is initialized for this thread. IFileOpenDialog and IShellItem are
        // valid COM objects. CoTaskMemFree frees memory allocated by GetDisplayName.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            let result = {
                let dialog: IFileOpenDialog =
                    match CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL) {
                        Ok(d) => d,
                        Err(e) => {
                            tracing::warn!("CoCreateInstance(FileOpenDialog) failed: {e}");
                            return None;
                        }
                    };

                if let Err(e) = dialog.SetOptions(FOS_PICKFOLDERS) {
                    tracing::warn!("SetOptions failed: {e}");
                }
                let title_wide = to_wide(title);
                if let Err(e) = dialog.SetTitle(PCWSTR(title_wide.as_ptr())) {
                    tracing::warn!("SetTitle failed: {e}");
                }

                if dialog.Show(Some(self.hwnd)).is_err() {
                    return None;
                }

                let item: IShellItem = match dialog.GetResult() {
                    Ok(i) => i,
                    Err(_) => return None,
                };

                let path_ptr = match item.GetDisplayName(SIGDN_FILESYSPATH) {
                    Ok(p) => p,
                    Err(_) => return None,
                };

                let path = path_ptr.to_string().inspect_err(|e| tracing::warn!("Path conversion failed: {e}")).ok();
                windows::Win32::System::Com::CoTaskMemFree(Some(path_ptr.0 as *const _));
                path
            };

            CoUninitialize();
            result
        }
    }

    /// DLL 파일 브라우저 열기
    fn browse_dll_file(&self, title: &str) -> Option<String> {
        use windows::Win32::System::Com::{
            CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
        };
        use windows::Win32::UI::Shell::{
            FileOpenDialog, IFileOpenDialog, IShellItem, SIGDN_FILESYSPATH,
        };
        use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;

        // SAFETY: COM is initialized for this thread. IFileOpenDialog and IShellItem are
        // valid COM objects. Filter strings are valid null-terminated UTF-16.
        // CoTaskMemFree frees memory allocated by GetDisplayName.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            let result = {
                let dialog: IFileOpenDialog =
                    match CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL) {
                        Ok(d) => d,
                        Err(e) => {
                            tracing::warn!("CoCreateInstance(FileOpenDialog) failed: {e}");
                            return None;
                        }
                    };

                let title_wide = to_wide(title);
                if let Err(e) = dialog.SetTitle(PCWSTR(title_wide.as_ptr())) {
                    tracing::warn!("SetTitle failed: {e}");
                }

                let filter_name = to_wide("DLL 파일");
                let filter_spec = to_wide("*.dll");
                let filters = [COMDLG_FILTERSPEC {
                    pszName: PCWSTR(filter_name.as_ptr()),
                    pszSpec: PCWSTR(filter_spec.as_ptr()),
                }];
                if let Err(e) = dialog.SetFileTypes(&filters) {
                    tracing::warn!("SetFileTypes failed: {e}");
                }

                if dialog.Show(Some(self.hwnd)).is_err() {
                    return None;
                }

                let item: IShellItem = match dialog.GetResult() {
                    Ok(i) => i,
                    Err(_) => return None,
                };

                let path_ptr = match item.GetDisplayName(SIGDN_FILESYSPATH) {
                    Ok(p) => p,
                    Err(_) => return None,
                };

                let path = path_ptr.to_string().inspect_err(|e| tracing::warn!("Path conversion failed: {e}")).ok();
                windows::Win32::System::Com::CoTaskMemFree(Some(path_ptr.0 as *const _));
                path
            };

            CoUninitialize();
            result
        }
    }

    /// 텍스트 크기 UI 업데이트 (트랙바 위치 및 레이블)
    fn update_textsize_ui(&self, size: i32) {
        // SAFETY: self.hwnd is valid; GetDlgItem returns valid control handles.
        unsafe {
            if let Ok(trackbar) = GetDlgItem(Some(self.hwnd), ctrl_id::TEXTSIZE_TRACKBAR as i32) {
                if !trackbar.is_invalid() {
                    let _ = SendMessageW(
                        trackbar,
                        TBM_SETPOS,
                        Some(WPARAM(1)),
                        Some(LPARAM(size as isize)),
                    );
                }
            }
            if let Ok(label) = GetDlgItem(Some(self.hwnd), ctrl_id::TEXTSIZE_TEXT as i32) {
                if !label.is_invalid() {
                    let text = format!("크기: {}", size);
                    let text_wide = to_wide(&text);
                    let _ = SetWindowTextW(label, PCWSTR(text_wide.as_ptr()));
                }
            }
        }
    }

    /// 설정 변경 알림
    pub(super) fn notify_change(&self) {
        if let Some(ref cb) = self.on_change {
            cb(&self.config.borrow());
        }

        if let Err(e) = self.config.borrow().save() {
            tracing::error!("설정 저장 실패: {}", e);
        }

        // SAFETY: self.main_hwnd is a valid window handle passed during dialog creation.
        unsafe {
            let _ = PostMessageW(Some(self.main_hwnd), WM_PAINT, WPARAM(0), LPARAM(1));
        }
    }
}
