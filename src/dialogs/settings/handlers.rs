//! 설정 대화상자 명령/이벤트 핸들러

use windows::{
    Win32::{
        Foundation::*, UI::Controls::*, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::ctrl_id;
use super::SettingsDialog;
use crate::config::{ColorType, TextAlign, TextType};
use crate::util::to_wide;
use crate::dialogs::color::ColorDialog;
use crate::dialogs::font::{FontDialog, FontDialogConfig, FontStyle};

/// +/- 버튼 처리 매크로: config에서 값을 읽고, 범위 내에서 증감 후, UI 업데이트
macro_rules! handle_size_button {
    ($self:expr, $get_field:expr, $set_color_type:expr, $delta:expr, $min:expr, $max:expr, $ui_update:expr) => {{
        let new_size = {
            let mut cfg = $self.config.borrow_mut();
            let current = $get_field(&cfg);
            let next = (current + $delta).clamp($min, $max);
            if next != current {
                cfg.set_all_text_size($set_color_type, next);
            }
            next
        };
        $ui_update($self, new_size);
        $self.notify_change();
    }};
}

/// 체크박스 토글 매크로: config 필드를 반전시키고 notify_change 호출
macro_rules! toggle_field {
    ($self:expr, $field:ident) => {{
        let mut cfg = $self.config.borrow_mut();
        cfg.$field = !cfg.$field;
        drop(cfg);
        $self.notify_change();
    }};
}

impl SettingsDialog {
    /// 명령 처리
    pub(super) fn handle_command(&mut self, cmd: u16, notify_code: u32) {
        // ComboBox 선택 변경은 별도 처리
        if notify_code == CBN_SELCHANGE {
            self.handle_combobox(cmd);
            return;
        }

        // Edit 컨트롤 포커스 해제 시 값 저장
        if notify_code == EN_KILLFOCUS {
            self.handle_edit_killfocus(cmd);
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
                    self.invalidate_color_button(BACKGROUND_COLOR);
                    self.notify_change();
                }
            }

            // 배경 표시 토글
            BACKGROUND_SWITCH => {
                self.config.borrow_mut().toggle_background_visible();
                self.notify_change();
            }

            // NAME/ORG/TRANS 색상 버튼
            NAME_COLOR => self.handle_color_button(NAME_COLOR, TextType::Name, ColorType::Primary),
            NAME_OUTLINE1 => self.handle_color_button(NAME_OUTLINE1, TextType::Name, ColorType::Outline1),
            NAME_OUTLINE2 => self.handle_color_button(NAME_OUTLINE2, TextType::Name, ColorType::Outline2),
            NAME_SHADOW_COLOR => self.handle_color_button(NAME_SHADOW_COLOR, TextType::Name, ColorType::Shadow),
            NAME_FONT => self.handle_font_button(TextType::Name),
            NAME_SHADOW => { self.config.borrow_mut().toggle_shadow(TextType::Name); self.notify_change(); }

            ORG_COLOR => self.handle_color_button(ORG_COLOR, TextType::Original, ColorType::Primary),
            ORG_OUTLINE1 => self.handle_color_button(ORG_OUTLINE1, TextType::Original, ColorType::Outline1),
            ORG_OUTLINE2 => self.handle_color_button(ORG_OUTLINE2, TextType::Original, ColorType::Outline2),
            ORG_SHADOW_COLOR => self.handle_color_button(ORG_SHADOW_COLOR, TextType::Original, ColorType::Shadow),
            ORG_FONT => self.handle_font_button(TextType::Original),
            ORG_SHADOW => { self.config.borrow_mut().toggle_shadow(TextType::Original); self.notify_change(); }

            TRANS_COLOR => self.handle_color_button(TRANS_COLOR, TextType::Translation, ColorType::Primary),
            TRANS_OUTLINE1 => self.handle_color_button(TRANS_OUTLINE1, TextType::Translation, ColorType::Outline1),
            TRANS_OUTLINE2 => self.handle_color_button(TRANS_OUTLINE2, TextType::Translation, ColorType::Outline2),
            TRANS_SHADOW_COLOR => self.handle_color_button(TRANS_SHADOW_COLOR, TextType::Translation, ColorType::Shadow),
            TRANS_FONT => self.handle_font_button(TextType::Translation),
            TRANS_SHADOW => { self.config.borrow_mut().toggle_shadow(TextType::Translation); self.notify_change(); }

            // 테두리 설정
            BORDER_MODE => {
                self.config.borrow_mut().toggle_border_visible();
                self.notify_change();
            }
            BORDER_COLOR => {
                let initial = self.config.borrow().border_color;
                if let Some(result) = ColorDialog::show_simple(self.hwnd, initial) {
                    self.config.borrow_mut().border_color = result.argb;
                    self.invalidate_color_button(BORDER_COLOR);
                    self.notify_change();
                }
            }

            // 표시 옵션 체크박스
            PRINT_ORGTEXT => toggle_field!(self, show_original),
            PRINT_TRANSTEXT => toggle_field!(self, show_translation),
            PRINT_ORGNAME => toggle_field!(self, show_name),
            SEPERATE_NAME => toggle_field!(self, separate_name),
            REPEAT_TEXT => {
                let mut cfg = self.config.borrow_mut();
                cfg.repeat_text_mode = (cfg.repeat_text_mode + 1) % 5;
                let new_mode = cfg.repeat_text_mode;
                drop(cfg);
                self.set_control_text(REPEAT_TEXT, &super::repeat_mode_label(new_mode));
                self.notify_change();
            }

            // 텍스트 정렬
            TEXTALIGN_LEFT => { self.config.borrow_mut().text_align = TextAlign::Left; self.notify_change(); }
            TEXTALIGN_MID => { self.config.borrow_mut().text_align = TextAlign::Center; self.notify_change(); }
            TEXTALIGN_RIGHT => { self.config.borrow_mut().text_align = TextAlign::Right; self.notify_change(); }

            // 윈도우 옵션 체크박스
            TOPMOST => toggle_field!(self, window_topmost),
            USE_MAGNETIC => { self.config.borrow_mut().toggle_magnetic_mode(); self.notify_change(); }
            MAGNETIC_MINIMIZE => toggle_field!(self, magnetic_minimize),
            HIDEWIN => toggle_field!(self, temp_window_hide),
            CLIPBOARD_WATCH => { self.config.borrow_mut().toggle_clipboard_watch(); self.notify_change(); }
            WNDCLICK_THROUGH => { self.config.borrow_mut().toggle_click_through(); self.notify_change(); }

            // 텍스트 크기 +/-
            TEXTSIZE_MINUS => handle_size_button!(self,
                |cfg: &crate::config::Config| cfg.translation_style.size,
                ColorType::Primary, -1, 6, 100,
                |s: &Self, v| s.update_textsize_ui(v)),
            TEXTSIZE_PLUS => handle_size_button!(self,
                |cfg: &crate::config::Config| cfg.translation_style.size,
                ColorType::Primary, 1, 6, 100,
                |s: &Self, v| s.update_textsize_ui(v)),

            // 외곽선1 +/-
            OUTLINE1_MINUS => handle_size_button!(self,
                |cfg: &crate::config::Config| cfg.translation_style.outline1_size,
                ColorType::Outline1, -1, 0, 20,
                |s: &Self, v| s.update_trackbar_pos(OUTLINE1_TRACKBAR, v)),
            OUTLINE1_PLUS => handle_size_button!(self,
                |cfg: &crate::config::Config| cfg.translation_style.outline1_size,
                ColorType::Outline1, 1, 0, 20,
                |s: &Self, v| s.update_trackbar_pos(OUTLINE1_TRACKBAR, v)),

            // 외곽선2 +/-
            OUTLINE2_MINUS => handle_size_button!(self,
                |cfg: &crate::config::Config| cfg.translation_style.outline2_size,
                ColorType::Outline2, -1, 0, 20,
                |s: &Self, v| s.update_trackbar_pos(OUTLINE2_TRACKBAR, v)),
            OUTLINE2_PLUS => handle_size_button!(self,
                |cfg: &crate::config::Config| cfg.translation_style.outline2_size,
                ColorType::Outline2, 1, 0, 20,
                |s: &Self, v| s.update_trackbar_pos(OUTLINE2_TRACKBAR, v)),

            // 자동 언어 감지
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
                    self.set_control_text(EZTRANS_DLL_EDIT, &path);
                    self.sync_translation_manager();
                    self.notify_change();
                }
            }

            // EzTrans Dat 폴더 찾아보기
            EZTRANS_DAT_BROWSE => {
                if let Some(path) = self.browse_folder_with_title("EzTrans Dat 폴더 선택") {
                    self.config.borrow_mut().translation.eztrans_dat_path = path.clone();
                    self.set_control_text(EZTRANS_DAT_EDIT, &path);
                    self.sync_translation_manager();
                    self.notify_change();
                }
            }

            // DeepL 보조 키 추가
            DEEPL_KEY_ADD_BTN => self.deepl_keys_add(),
            // DeepL 보조 키 삭제
            DEEPL_KEY_REMOVE_BTN => self.deepl_keys_remove(),

            // 글로서리 편집 다이얼로그
            LLM_GLOSSARY_EDIT_BTN => self.open_glossary_editor(),

            _ => {}
        }
    }

    /// DeepL 보조 키 추가 — 입력 Edit 내용을 리스트박스에 추가하고 Config 동기화
    fn deepl_keys_add(&mut self) {
        let key = self.get_control_text(ctrl_id::DEEPL_KEY_ADD_EDIT);
        let key = key.trim().to_string();
        if key.is_empty() {
            return;
        }
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid listbox.
        unsafe {
            let listbox = match GetDlgItem(Some(self.hwnd), ctrl_id::DEEPL_KEYS_LIST as i32) {
                Ok(h) if !h.is_invalid() => h,
                _ => return,
            };
            let key_wide = to_wide(&key);
            let _ = SendMessageW(
                listbox,
                LB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(key_wide.as_ptr() as isize)),
            );
        }
        self.config.borrow_mut().translation.deepl_keys.push(key);
        self.set_control_text(ctrl_id::DEEPL_KEY_ADD_EDIT, "");
        self.notify_change();
    }

    /// DeepL 보조 키 삭제 — 선택된 항목 제거 및 Config 동기화
    fn deepl_keys_remove(&mut self) {
        // SAFETY: dialog hwnd is valid; GetDlgItem returns a valid listbox.
        let sel = unsafe {
            let listbox = match GetDlgItem(Some(self.hwnd), ctrl_id::DEEPL_KEYS_LIST as i32) {
                Ok(h) if !h.is_invalid() => h,
                _ => return,
            };
            let sel = SendMessageW(listbox, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32;
            if sel == LB_ERR { return; }
            let _ = SendMessageW(
                listbox,
                LB_DELETESTRING,
                Some(WPARAM(sel as usize)),
                Some(LPARAM(0)),
            );
            sel
        };
        {
            let mut cfg = self.config.borrow_mut();
            if (sel as usize) < cfg.translation.deepl_keys.len() {
                cfg.translation.deepl_keys.remove(sel as usize);
            }
        }
        self.notify_change();
    }

    /// 글로서리 편집기 다이얼로그 열기
    fn open_glossary_editor(&mut self) {
        let config = self.config.clone();
        let _ = crate::dialogs::glossary::GlossaryDialog::show(self.hwnd, config.clone());
        // 다이얼로그가 닫힌 후 표시 라벨 갱신
        let count = config.borrow().translation.llm.glossary.len();
        self.set_control_text(ctrl_id::LLM_GLOSSARY_COUNT_LABEL, &format!("사전 항목: {}", count));
    }

    /// 색상 버튼 처리
    fn handle_color_button(&mut self, ctrl_id: u16, text_type: TextType, color_type: ColorType) {
        let initial = self.config.borrow().get_text_color(text_type, color_type);
        if let Some(result) = ColorDialog::show_simple(self.hwnd, initial) {
            self.config
                .borrow_mut()
                .set_text_color(text_type, color_type, result.argb);
            self.invalidate_color_button(ctrl_id);
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
            }
            TEXTSIZE_TRACKBAR => {
                self.config.borrow_mut().set_all_text_size(ColorType::Primary, value);
                self.set_control_text(TEXTSIZE_TEXT, &format!("크기: {}", value));
            }
            OUTLINE1_TRACKBAR => {
                self.config.borrow_mut().set_all_text_size(ColorType::Outline1, value);
            }
            OUTLINE2_TRACKBAR => {
                self.config.borrow_mut().set_all_text_size(ColorType::Outline2, value);
            }
            SHADOW_X_TRACKBAR => { self.config.borrow_mut().shadow_offset_x = value; }
            SHADOW_Y_TRACKBAR => { self.config.borrow_mut().shadow_offset_y = value; }
            MARGIN_X_TRACKBAR => { self.config.borrow_mut().text_margin_x = value; }
            MARGIN_Y_TRACKBAR => { self.config.borrow_mut().text_margin_y = value; }
            MARGIN_NAME_TRACKBAR => { self.config.borrow_mut().name_margin = value; }
            BORDER_SIZE_TRACKBAR => { self.config.borrow_mut().border_width = value; }
            LLM_TEMPERATURE_TRACKBAR => {
                let temp = (value as f32 / 100.0).clamp(0.0, 2.0);
                self.config.borrow_mut().translation.llm.temperature = temp;
                self.set_control_text(LLM_TEMPERATURE_LABEL, &format!("{:.2}", temp));
            }
            _ => return,
        }
        self.notify_change();
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
                    // 엔진 변경 시 해당 그룹만 활성화하고 언어 콤보 항목 재구성
                    self.apply_engine_state(engine);
                }
                TRANS_SOURCE_LANG => {
                    let engine = self.config.borrow().translation.get_engine();
                    self.config.borrow_mut().translation.set_source_lang_by_index(sel, engine);
                }
                TRANS_TARGET_LANG => {
                    let engine = self.config.borrow().translation.get_engine();
                    self.config.borrow_mut().translation.set_target_lang_by_index(sel, engine);
                }
                LLM_PROVIDER => {
                    use crate::translation::LlmProvider;
                    let provider = LlmProvider::from_u8(sel as u8);
                    self.config.borrow_mut().translation.llm.set_provider(provider);
                }
                DEEPL_STRATEGY_COMBO => {
                    let strategy = if sel == 1 { "round-robin" } else { "failover" };
                    self.config.borrow_mut().translation.deepl_strategy = strategy.to_string();
                }
                _ => return,
            }
            self.sync_translation_manager();
            self.notify_change();
        }
    }

    /// EzTrans 초기화 동기화 (다른 엔진은 워커가 매번 자격증명을 받아 stateless)
    fn sync_translation_manager(&self) {
        use crate::translation::get_eztrans_manager;
        let config = self.config.borrow();
        if config.translation.eztrans_dll_path.is_empty()
            || config.translation.eztrans_dat_path.is_empty()
        {
            return;
        }
        let manager = get_eztrans_manager();
        if let Ok(mut mgr) = manager.lock() {
            if let Err(e) = mgr.init(
                &config.translation.eztrans_dll_path,
                &config.translation.eztrans_dat_path,
            ) {
                tracing::warn!("EzTrans init failed in sync: {e}");
            }
        }
    }

    /// 컨트롤 텍스트 설정 헬퍼
    fn set_control_text(&self, ctrl_id: u16, text: &str) {
        // SAFETY: self.hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            if let Ok(ctrl) = GetDlgItem(Some(self.hwnd), ctrl_id as i32) {
                if !ctrl.is_invalid() {
                    let text_wide = to_wide(text);
                    let _ = SetWindowTextW(ctrl, PCWSTR(text_wide.as_ptr()));
                }
            }
        }
    }

    /// 제목 지정 폴더 브라우저 열기
    fn browse_folder_with_title(&self, title: &str) -> Option<String> {
        crate::dialogs::file_dialog::pick_folder(self.hwnd, title)
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// DLL 파일 브라우저 열기
    fn browse_dll_file(&self, title: &str) -> Option<String> {
        let filters = [crate::dialogs::file_dialog::FileFilter {
            name: "DLL 파일",
            spec: "*.dll",
        }];
        crate::dialogs::file_dialog::open_file(self.hwnd, title, &filters)
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// Edit 컨트롤 포커스 해제 시 값 저장
    fn handle_edit_killfocus(&mut self, ctrl_id: u16) {
        let text = self.get_control_text(ctrl_id);
        use ctrl_id::*;
        match ctrl_id {
            DEEPL_API_KEY_EDIT => {
                self.config.borrow_mut().translation.deepl_api_key = text;
                self.sync_translation_manager();
                self.notify_change();
            }
            PAPAGO_ID_EDIT => {
                self.config.borrow_mut().translation.papago_client_id = text;
                self.sync_translation_manager();
                self.notify_change();
            }
            PAPAGO_SECRET_EDIT => {
                self.config.borrow_mut().translation.papago_client_secret = text;
                self.sync_translation_manager();
                self.notify_change();
            }
            EZTRANS_DLL_EDIT => {
                self.config.borrow_mut().translation.eztrans_dll_path = text;
                self.sync_translation_manager();
                self.notify_change();
            }
            EZTRANS_DAT_EDIT => {
                self.config.borrow_mut().translation.eztrans_dat_path = text;
                self.sync_translation_manager();
                self.notify_change();
            }
            LLM_MODEL_EDIT => {
                self.config.borrow_mut().translation.llm.model = text;
                self.notify_change();
            }
            LLM_API_KEY_EDIT => {
                self.config.borrow_mut().translation.llm.api_key = text;
                self.sync_translation_manager();
                self.notify_change();
            }
            LLM_BASE_URL_EDIT => {
                self.config.borrow_mut().translation.llm.base_url = text;
                self.notify_change();
            }
            LLM_SYSTEM_PROMPT_EDIT => {
                self.config.borrow_mut().translation.llm.system_prompt = text;
                self.notify_change();
            }
            LLM_MAX_TOKENS_EDIT => {
                if let Ok(v) = text.trim().parse::<u32>() {
                    self.config.borrow_mut().translation.llm.max_tokens = v.clamp(1, 32_000);
                    self.notify_change();
                }
            }
            LLM_DEBOUNCE_EDIT => {
                if let Ok(v) = text.trim().parse::<u32>() {
                    self.config.borrow_mut().translation.llm.debounce_ms = v.clamp(0, 10_000);
                    self.notify_change();
                }
            }
            _ => {}
        }
    }

    /// Edit 컨트롤에서 텍스트 가져오기
    fn get_control_text(&self, ctrl_id: u16) -> String {
        // SAFETY: self.hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            let ctrl = match GetDlgItem(Some(self.hwnd), ctrl_id as i32) {
                Ok(h) if !h.is_invalid() => h,
                _ => return String::new(),
            };
            let len = GetWindowTextLengthW(ctrl);
            if len == 0 { return String::new(); }
            let mut buffer: Vec<u16> = vec![0; (len + 1) as usize];
            GetWindowTextW(ctrl, &mut buffer);
            String::from_utf16_lossy(&buffer[..len as usize])
        }
    }

    /// 트랙바 위치 업데이트
    fn update_trackbar_pos(&self, trackbar_id: u16, value: i32) {
        // SAFETY: self.hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            if let Ok(trackbar) = GetDlgItem(Some(self.hwnd), trackbar_id as i32) {
                if !trackbar.is_invalid() {
                    let _ = SendMessageW(
                        trackbar,
                        TBM_SETPOS,
                        Some(WPARAM(1)),
                        Some(LPARAM(value as isize)),
                    );
                }
            }
        }
    }

    /// 텍스트 크기 UI 업데이트 (트랙바 위치 및 레이블)
    fn update_textsize_ui(&self, size: i32) {
        self.update_trackbar_pos(ctrl_id::TEXTSIZE_TRACKBAR, size);
        self.set_control_text(ctrl_id::TEXTSIZE_TEXT, &format!("크기: {}", size));
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
