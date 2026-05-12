//! 번역 대화상자
//!
//! 수동 번역 입력을 위한 대화상자.
//! Edit 컨트롤 서브클래싱(Comctl32 v6 `SetWindowSubclass`)으로 Ctrl+A 전체 선택 지원.
//! 번역 엔진 선택 (EzTrans, Google, DeepL) 및 언어 선택 지원.

use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::DataExchange::*,
        System::LibraryLoader::GetModuleHandleW, System::Memory::*,
        System::Ole::CF_UNICODETEXT, UI::Controls::*,
        UI::Input::KeyboardAndMouse::*,
        UI::Shell::{DefSubclassProc, SetWindowSubclass},
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::util::to_wide;
use crate::define_dialog_instance;
use super::helpers::{Dialog, DialogControls};

use crate::config::Config;
use crate::constants::WM_TRANSLATION_COMPLETE;
use crate::translation::{
    get_eztrans_manager, request_translation, take_response, unregister_translation_hwnd,
    Language, LlmProvider, TranslationEngine,
};

// 컨트롤 ID
mod ctrl_id {
    pub const SOURCE_EDIT: u16 = 2001;
    pub const DEST_EDIT: u16 = 2002;
    pub const BTN_TRANSLATE: u16 = 2003;
    pub const BTN_COPY: u16 = 2004;
    pub const BTN_CLEAR: u16 = 2005;
    pub const CHK_ONE_GO: u16 = 2006;
    pub const CHK_NO_LINEFEED: u16 = 2007;
    pub const RADIO_OUTPUT_1: u16 = 2010;
    pub const RADIO_OUTPUT_2: u16 = 2011;
    pub const RADIO_OUTPUT_3: u16 = 2012;
    // 번역 엔진 선택
    pub const COMBO_ENGINE: u16 = 2020;
    pub const COMBO_SOURCE_LANG: u16 = 2021;
    pub const COMBO_TARGET_LANG: u16 = 2022;
    // LLM 하위 설정 (엔진이 LLM일 때만 노출)
    pub const COMBO_LLM_PROVIDER: u16 = 2030;
    pub const EDIT_LLM_MODEL: u16 = 2031;
    pub const EDIT_LLM_API_KEY: u16 = 2032;
}

// 서브클래스 ID (uIdSubclass): 컨트롤별로 구분
mod subclass_id {
    pub const SOURCE_EDIT: usize = 1;
    pub const DEST_EDIT: usize = 2;
}

/// 출력 형식
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Normal = 0, // 일반
    Brackets = 1,  // 괄호 포함
    NameSplit = 2, // 이름 분리
}

/// 번역 대화상자
pub struct TranslateDialog {
    hwnd: HWND,
    config: Rc<RefCell<Config>>,
    source_edit: HWND,
    dest_edit: HWND,
    engine_combo: HWND,
    source_lang_combo: HWND,
    target_lang_combo: HWND,
    /// LLM 하위 컨트롤. 엔진이 LLM일 때만 visible 처리.
    llm_group: HWND,
    llm_provider_label: HWND,
    llm_provider_combo: HWND,
    llm_model_label: HWND,
    llm_model_edit: HWND,
    llm_api_key_label: HWND,
    llm_api_key_edit: HWND,
    one_go: bool,
    no_linefeed: bool,
    output_format: OutputFormat,
    engine_initialized: bool,
    /// 번역 진행 중 여부
    translating: bool,
}

impl DialogControls for TranslateDialog {
    fn dialog_hwnd(&self) -> HWND { self.hwnd }
}

define_dialog_instance!(TRANSLATE_INSTANCE: TranslateDialog);

impl Dialog for TranslateDialog {
    type Params = Rc<RefCell<Config>>;

    const CLASS_NAME: PCWSTR = w!("AnemoneTranslateClass");
    const TITLE: PCWSTR = w!("번역");
    const WIDTH: i32 = 510;
    // LLM 그룹(엔진이 LLM일 때만 노출) 자리를 옵션/동작 아래에 확보. 비-LLM 엔진에서도
    // 다이얼로그 높이는 동일하지만 LLM 그룹 컨트롤은 hide 처리한다.
    const HEIGHT: i32 = 540;
    const EXTRA_STYLE: WINDOW_STYLE = WINDOW_STYLE(0);

    fn instance_slot()
        -> &'static std::thread::LocalKey<
            std::cell::RefCell<Option<std::rc::Rc<std::cell::RefCell<Self>>>>,
        > {
        &TRANSLATE_INSTANCE
    }

    fn init(hwnd: HWND, _parent: HWND, config: Self::Params) -> Self {
        TranslateDialog {
            hwnd,
            config,
            source_edit: HWND::default(),
            dest_edit: HWND::default(),
            engine_combo: HWND::default(),
            source_lang_combo: HWND::default(),
            target_lang_combo: HWND::default(),
            llm_group: HWND::default(),
            llm_provider_label: HWND::default(),
            llm_provider_combo: HWND::default(),
            llm_model_label: HWND::default(),
            llm_model_edit: HWND::default(),
            llm_api_key_label: HWND::default(),
            llm_api_key_edit: HWND::default(),
            one_go: false,
            no_linefeed: false,
            output_format: OutputFormat::Normal,
            engine_initialized: false,
            translating: false,
        }
    }

    fn create_controls(&mut self) -> Result<()> {
        // SAFETY: self.hwnd is a valid window handle from show_impl. All CreateWindowExW
        // and SendMessageW calls use valid handles. SetWindowSubclass installs a Comctl32
        // subclass for the edit controls and is auto-cleaned up on control destruction.
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let hfont = GetStockObject(DEFAULT_GUI_FONT);
            let dpi = crate::dpi::dpi_for_window(self.hwnd);
            let s = |v: i32| crate::dpi::scale(v, dpi);

            // ====== 번역 엔진 선택 그룹 ======
            self.create_group_box(10, 5, 475, 55, "번역 설정")?;

            self.create_label(20, 28, 40, 18, "엔진:")?;
            self.engine_combo = DialogControls::create_combobox(self, 65, 25, 100, 150, ctrl_id::COMBO_ENGINE, &[], 0)?;
            self.add_combobox_item(self.engine_combo, "EzTrans");
            self.add_combobox_item(self.engine_combo, "Google");
            self.add_combobox_item(self.engine_combo, "DeepL");
            self.add_combobox_item(self.engine_combo, "Papago");
            self.add_combobox_item(self.engine_combo, "LLM");

            self.create_label(180, 28, 40, 18, "소스:")?;
            self.source_lang_combo =
                DialogControls::create_combobox(self, 220, 25, 100, 150, ctrl_id::COMBO_SOURCE_LANG, &[], 0)?;

            self.create_label(335, 28, 40, 18, "타겟:")?;
            self.target_lang_combo =
                DialogControls::create_combobox(self, 375, 25, 100, 150, ctrl_id::COMBO_TARGET_LANG, &[], 0)?;

            // 설정에서 초기값 로드
            {
                let config = self.config.borrow();
                let engine = config.translation.get_engine();
                let _ = SendMessageW(
                    self.engine_combo, CB_SETCURSEL,
                    Some(WPARAM(config.translation.engine_as_u8() as usize)), None,
                );
                self.populate_language_combos(engine);
                let _ = SendMessageW(
                    self.source_lang_combo, CB_SETCURSEL,
                    Some(WPARAM(config.translation.source_lang_index(engine))), None,
                );
                let _ = SendMessageW(
                    self.target_lang_combo, CB_SETCURSEL,
                    Some(WPARAM(config.translation.target_lang_index(engine))), None,
                );
            }

            // ====== 원문 입력 그룹 ======
            self.create_group_box(10, 65, 475, 130, "원문 입력")?;

            self.source_edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("EDIT"), w!(""),
                WINDOW_STYLE(
                    WS_CHILD.0 | WS_VISIBLE.0 | WS_VSCROLL.0
                        | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_WANTRETURN as u32,
                ),
                s(20), s(85), s(455), s(100),
                Some(self.hwnd),
                Some(HMENU(ctrl_id::SOURCE_EDIT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let _ = SendMessageW(self.source_edit, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));
            let _ = SendMessageW(self.source_edit, EM_SETLIMITTEXT, Some(WPARAM(0)), Some(LPARAM(0)));

            // 서브클래싱 (Comctl32 v6 SetWindowSubclass)
            let _ = SetWindowSubclass(
                self.source_edit,
                Some(Self::edit_subclass_proc),
                subclass_id::SOURCE_EDIT,
                0,
            );

            // ====== 번역 결과 그룹 ======
            self.create_group_box(10, 200, 475, 130, "번역 결과")?;

            self.dest_edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("EDIT"), w!(""),
                WINDOW_STYLE(
                    WS_CHILD.0 | WS_VISIBLE.0 | WS_VSCROLL.0
                        | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_READONLY as u32,
                ),
                s(20), s(220), s(455), s(100),
                Some(self.hwnd),
                Some(HMENU(ctrl_id::DEST_EDIT as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let _ = SendMessageW(self.dest_edit, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));
            let _ = SendMessageW(self.dest_edit, EM_SETLIMITTEXT, Some(WPARAM(0)), Some(LPARAM(0)));

            // 서브클래싱 (Comctl32 v6 SetWindowSubclass)
            let _ = SetWindowSubclass(
                self.dest_edit,
                Some(Self::edit_subclass_proc),
                subclass_id::DEST_EDIT,
                0,
            );

            // ====== 옵션 그룹 ======
            self.create_group_box(10, 335, 230, 90, "옵션")?;

            self.create_checkbox(20, 355, 100, 20, ctrl_id::CHK_ONE_GO, "자동 번역", self.one_go)?;
            self.create_checkbox(125, 355, 110, 20, ctrl_id::CHK_NO_LINEFEED, "줄바꿈 제거", self.no_linefeed)?;

            self.create_label(20, 380, 70, 18, "출력 형식:")?;
            self.create_radio(95, 378, 50, 20, ctrl_id::RADIO_OUTPUT_1, "일반",
                self.output_format == OutputFormat::Normal)?;
            self.create_radio(150, 378, 50, 20, ctrl_id::RADIO_OUTPUT_2, "괄호",
                self.output_format == OutputFormat::Brackets)?;
            self.create_radio(205, 378, 50, 20, ctrl_id::RADIO_OUTPUT_3, "분리",
                self.output_format == OutputFormat::NameSplit)?;

            // ====== 버튼 그룹 ======
            self.create_group_box(250, 335, 235, 90, "동작")?;

            self.create_button(265, 360, 65, 28, ctrl_id::BTN_TRANSLATE, "번역")?;
            self.create_button(340, 360, 65, 28, ctrl_id::BTN_COPY, "복사")?;
            self.create_button(415, 360, 60, 28, ctrl_id::BTN_CLEAR, "초기화")?;

            // ====== LLM 하위 설정 그룹 (엔진이 LLM일 때만 노출) ======
            // 옵션/동작 그룹 아래(y=430)에 한 줄로 배치. 줄1: 제공자 + 모델, 줄2: API 키.
            self.llm_group = self.create_group_box(10, 430, 475, 100, "LLM 설정")?;

            self.llm_provider_label = self.create_label(20, 453, 50, 18, "제공자:")?;
            self.llm_provider_combo = DialogControls::create_combobox(
                self, 75, 450, 110, 180, ctrl_id::COMBO_LLM_PROVIDER, &[], 0,
            )?;
            for p in LlmProvider::ALL {
                self.add_combobox_item(self.llm_provider_combo, p.display_name());
            }

            self.llm_model_label = self.create_label(200, 453, 40, 18, "모델:")?;
            let model_text = self.config.borrow().translation.llm.model.clone();
            self.llm_model_edit = self.create_edit(
                240, 450, 235, 22, ctrl_id::EDIT_LLM_MODEL, &model_text,
            )?;

            self.llm_api_key_label = self.create_label(20, 488, 60, 18, "API 키:")?;
            let api_key_text = self.config.borrow().translation.llm.api_key.clone();
            self.llm_api_key_edit = self.create_edit(
                85, 485, 390, 22, ctrl_id::EDIT_LLM_API_KEY, &api_key_text,
            )?;

            // 제공자 콤보 초기 선택
            let provider_sel = self.config.borrow().translation.llm.get_provider() as u8 as usize;
            let _ = SendMessageW(
                self.llm_provider_combo, CB_SETCURSEL,
                Some(WPARAM(provider_sel)), None,
            );

            // 엔진 상태에 맞춰 초기 표시/숨김
            let engine = self.config.borrow().translation.get_engine();
            self.update_llm_group_visibility(engine);

            Ok(())
        }
    }

    /// 커스텀 메시지 핸들러
    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
        if msg == WM_TRANSLATION_COMPLETE {
            self.handle_translation_complete(wparam.0 as u64);
            return Some(LRESULT(0));
        }

        // 다이얼로그가 사라질 때 전역 번역 디스패치의 라우팅/대기 응답 정리.
        // None 을 반환해 매크로 본체의 WM_DESTROY 처리( $INSTANCE 해제 )가 이어 실행되도록 한다.
        if msg == WM_DESTROY {
            unregister_translation_hwnd(self.hwnd);
            return None;
        }

        // WM_COMMAND에서 EN_CHANGE 자동번역 처리
        if msg == WM_COMMAND {
            let id = (wparam.0 & 0xFFFF) as u16;
            let notify_code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            if notify_code == EN_CHANGE && id == ctrl_id::SOURCE_EDIT {
                if self.one_go {
                    self.do_translate();
                }
                return Some(LRESULT(0));
            }
        }

        None
    }

    /// 명령 처리
    fn handle_command(&mut self, cmd: u16, notify_code: u32) {
        use ctrl_id::*;

        match cmd {
            BTN_TRANSLATE => self.do_translate(),
            BTN_COPY => self.copy_to_clipboard(),
            BTN_CLEAR => self.clear_text(),
            CHK_ONE_GO => self.one_go = !self.one_go,
            CHK_NO_LINEFEED => self.no_linefeed = !self.no_linefeed,
            RADIO_OUTPUT_1 => self.output_format = OutputFormat::Normal,
            RADIO_OUTPUT_2 => self.output_format = OutputFormat::Brackets,
            RADIO_OUTPUT_3 => self.output_format = OutputFormat::NameSplit,
            COMBO_ENGINE | COMBO_SOURCE_LANG | COMBO_TARGET_LANG => {
                // CBN_SELCHANGE
                if notify_code == 1 {
                    if cmd == COMBO_ENGINE {
                        // SAFETY: engine_combo is a valid handle.
                        let engine_idx = unsafe {
                            SendMessageW(self.engine_combo, CB_GETCURSEL, None, None).0 as u8
                        };
                        let engine = TranslationEngine::from_u8(engine_idx);
                        self.populate_language_combos(engine);
                        self.update_llm_group_visibility(engine);
                        self.engine_initialized = false;
                    }
                    self.apply_current_settings();
                }
            }
            COMBO_LLM_PROVIDER => {
                if notify_code == 1 {
                    self.apply_llm_provider();
                    self.engine_initialized = false;
                }
            }
            EDIT_LLM_MODEL => {
                if notify_code == EN_CHANGE {
                    self.apply_llm_model();
                }
            }
            EDIT_LLM_API_KEY => {
                if notify_code == EN_CHANGE {
                    self.apply_llm_api_key();
                }
            }
            _ => {}
        }
    }
}

impl TranslateDialog {
    /// 엔진에 맞게 언어 콤보박스 항목 갱신
    fn populate_language_combos(&self, engine: TranslationEngine) {
        use crate::translation::lang_utils;
        // SAFETY: combo handles are valid controls from create_controls.
        unsafe {
            let _ = SendMessageW(self.source_lang_combo, CB_RESETCONTENT, None, None);
            for &lang in engine.supported_source_languages() {
                self.add_combobox_item(self.source_lang_combo, lang_utils::to_korean_name(lang));
            }
            let _ = SendMessageW(self.source_lang_combo, CB_SETCURSEL, Some(WPARAM(0)), None);

            let _ = SendMessageW(self.target_lang_combo, CB_RESETCONTENT, None, None);
            for &lang in engine.supported_target_languages() {
                self.add_combobox_item(self.target_lang_combo, lang_utils::to_korean_name(lang));
            }
            let _ = SendMessageW(self.target_lang_combo, CB_SETCURSEL, Some(WPARAM(0)), None);
        }
    }

    /// 엔진이 LLM일 때만 LLM 그룹 노출.
    fn update_llm_group_visibility(&self, engine: TranslationEngine) {
        let show = if engine == TranslationEngine::Llm { SW_SHOW } else { SW_HIDE };
        // SAFETY: 모든 LLM 그룹 HWND는 create_controls에서 생성된 유효한 핸들.
        unsafe {
            let _ = ShowWindow(self.llm_group, show);
            let _ = ShowWindow(self.llm_provider_label, show);
            let _ = ShowWindow(self.llm_provider_combo, show);
            let _ = ShowWindow(self.llm_model_label, show);
            let _ = ShowWindow(self.llm_model_edit, show);
            let _ = ShowWindow(self.llm_api_key_label, show);
            let _ = ShowWindow(self.llm_api_key_edit, show);
        }
    }

    fn apply_llm_provider(&self) {
        // SAFETY: 콤보 핸들은 create_controls에서 만든 유효한 핸들.
        let sel = unsafe {
            SendMessageW(self.llm_provider_combo, CB_GETCURSEL, None, None).0 as u8
        };
        let provider = LlmProvider::from_u8(sel);
        self.config.borrow_mut().translation.llm.set_provider(provider);
    }

    fn apply_llm_model(&self) {
        // SAFETY: edit 핸들은 create_controls에서 만든 유효한 핸들.
        let text = unsafe { Self::get_edit_text(self.llm_model_edit) };
        self.config.borrow_mut().translation.llm.model = text;
    }

    fn apply_llm_api_key(&self) {
        // SAFETY: edit 핸들은 create_controls에서 만든 유효한 핸들.
        let text = unsafe { Self::get_edit_text(self.llm_api_key_edit) };
        self.config.borrow_mut().translation.llm.api_key = text;
    }

    fn add_combobox_item(&self, combo: HWND, text: &str) {
        // SAFETY: combo is a valid combobox handle. wide string is valid for the call.
        unsafe {
            let wide = to_wide(text);
            let _ = SendMessageW(
                combo, CB_ADDSTRING,
                None, Some(LPARAM(wide.as_ptr() as isize)),
            );
        }
    }

    /// Edit 서브클래스 프로시저 (Ctrl+A 지원)
    ///
    /// Comctl32 v6 `SetWindowSubclass`용 SUBCLASSPROC. `DefSubclassProc`가 다음 서브클래스/
    /// 원본 wndproc로 자동 체이닝해주고, 컨트롤 파괴 시 OS가 서브클래스를 정리한다.
    // SAFETY: This is a subclassed Win32 window procedure. The system provides valid params.
    unsafe extern "system" fn edit_subclass_proc(
        hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
        _uid_subclass: usize, _ref_data: usize,
    ) -> LRESULT {
        // SAFETY: hwnd is a valid edit control owned by this dialog while subclassed.
        unsafe {
            if msg == WM_KEYDOWN
                && wparam.0 == 'A' as usize
                && (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0
            {
                let _ = SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                return LRESULT(0);
            }

            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
    }

    /// 원문 텍스트 가져오기
    fn get_source_text(&self) -> String {
        // SAFETY: self.source_edit is a valid edit control handle from create_controls.
        unsafe { Self::get_edit_text(self.source_edit) }
    }

    /// Edit 컨트롤에서 텍스트 가져오기
    unsafe fn get_edit_text(hwnd: HWND) -> String {
        // SAFETY: hwnd is a valid edit control. GetWindowTextLengthW returns the text length,
        // and GetWindowTextW fills the buffer up to that length.
        unsafe {
            let len = GetWindowTextLengthW(hwnd);
            if len == 0 { return String::new(); }
            let mut buffer: Vec<u16> = vec![0; (len + 1) as usize];
            GetWindowTextW(hwnd, &mut buffer);
            String::from_utf16_lossy(&buffer[..len as usize])
        }
    }

    /// Edit 컨트롤에 텍스트 설정
    unsafe fn set_edit_text(hwnd: HWND, text: &str) {
        // SAFETY: hwnd is a valid edit control. wide string is valid for the call duration.
        unsafe {
            let wide = to_wide(text);
            let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
        }
    }

    /// 번역 결과 설정
    fn set_dest_text(&self, text: &str) {
        // SAFETY: self.dest_edit is a valid edit control handle from create_controls.
        unsafe { Self::set_edit_text(self.dest_edit, text) }
    }

    /// 번역 엔진 초기화
    fn init_translation_engine(&mut self) -> std::result::Result<(), String> {
        if self.engine_initialized { return Ok(()); }

        let config = self.config.borrow();
        let engine = config.translation.get_engine();

        if engine == TranslationEngine::EzTrans {
            if config.translation.eztrans_dll_path.is_empty()
                || config.translation.eztrans_dat_path.is_empty()
            {
                return Err("EzTrans 경로가 설정되지 않았습니다. 번역 설정에서 경로를 지정하세요.".to_string());
            }
            let manager = get_eztrans_manager();
            if let Ok(mut mgr) = manager.lock() {
                if let Err(e) = mgr.init(
                    &config.translation.eztrans_dll_path,
                    &config.translation.eztrans_dat_path,
                ) {
                    return Err(format!("EzTrans 초기화 실패: {}", e));
                }
            } else {
                return Err("EzTrans 매니저 잠금 실패".to_string());
            }
        }

        self.engine_initialized = true;
        Ok(())
    }

    /// 현재 선택된 엔진/언어를 매니저에 적용
    fn apply_current_settings(&self) {
        // SAFETY: combo handles are valid controls from create_controls. SendMessageW with
        // CB_GETCURSEL returns the current selection index.
        unsafe {
            let engine_idx = SendMessageW(self.engine_combo, CB_GETCURSEL, None, None).0 as usize;
            let source_idx = SendMessageW(self.source_lang_combo, CB_GETCURSEL, None, None).0 as usize;
            let target_idx = SendMessageW(self.target_lang_combo, CB_GETCURSEL, None, None).0 as usize;

            let engine = TranslationEngine::from_u8(engine_idx as u8);
            let supported_source = engine.supported_source_languages();
            let supported_target = engine.supported_target_languages();

            let source_lang = supported_source.get(source_idx).copied().unwrap_or(Language::Jpn);
            let target_lang = supported_target.get(target_idx).copied().unwrap_or(Language::Kor);

            let mut config = self.config.borrow_mut();
            config.translation.set_engine(engine);
            config.translation.set_source_language(source_lang);
            config.translation.set_target_language(target_lang);
        }
    }

    /// 번역 수행 (비동기)
    fn do_translate(&mut self) {
        let source = self.get_source_text();
        if source.is_empty() { return; }
        if self.translating { return; }

        if let Err(e) = self.init_translation_engine() {
            self.set_dest_text(&format!("[오류] {}", e));
            return;
        }

        self.apply_current_settings();

        let text = if self.no_linefeed {
            source.replace("\r\n", " ").replace('\n', " ")
        } else {
            source
        };

        let (engine, source_lang, target_lang, credentials) = {
            use crate::translation::EngineCredentials;
            let config = self.config.borrow();
            let engine = config.translation.get_engine();
            let source_lang = config.translation.get_source_language();
            let target_lang = config.translation.get_target_language();
            let credentials = match engine {
                TranslationEngine::DeepL => EngineCredentials::DeepL {
                    keys: config.translation.deepl_effective_keys(),
                    strategy: config.translation.deepl_strategy(),
                },
                TranslationEngine::Papago => EngineCredentials::Papago {
                    client_id: config.translation.papago_client_id.clone(),
                    client_secret: config.translation.papago_client_secret.clone(),
                },
                TranslationEngine::Llm => {
                    EngineCredentials::Llm(config.translation.llm.to_call_params())
                }
                _ => EngineCredentials::None,
            };
            (engine, source_lang, target_lang, credentials)
        };

        self.set_dest_text("[번역 중...]");
        self.translating = true;

        request_translation(self.hwnd, text, engine, source_lang, target_lang, credentials);
    }

    /// 번역 완료 처리. WPARAM 의 `req_id` 로 자신의 응답만 꺼낸다.
    fn handle_translation_complete(&mut self, req_id: u64) {
        let Some(response) = take_response(req_id) else {
            return;
        };

        self.translating = false;

        let result = match response.result {
            Ok(translated) => match self.output_format {
                OutputFormat::Normal => translated,
                OutputFormat::Brackets => format!("「{}」", translated),
                OutputFormat::NameSplit => {
                    if let Some((name, rest)) = translated.split_once([':', '：']) {
                        format!("{}\n{}", name.trim(), rest.trim())
                    } else {
                        translated
                    }
                }
            },
            Err(err) => format!("[오류] {}", err),
        };
        self.set_dest_text(&result);
    }

    /// 번역 결과를 클립보드에 복사
    fn copy_to_clipboard(&self) {
        // SAFETY: self.dest_edit and self.hwnd are valid handles from create_controls.
        unsafe {
            let text = Self::get_edit_text(self.dest_edit);
            if text.is_empty() { return; }
            Self::set_clipboard_text(&text, self.hwnd);
        }
    }

    /// 클립보드에 텍스트 설정
    unsafe fn set_clipboard_text(text: &str, hwnd: HWND) {
        // SAFETY: hwnd is a valid window handle. OpenClipboard/CloseClipboard are called in
        // matched pairs. GlobalAlloc/GlobalLock/GlobalUnlock manage clipboard memory.
        // copy_nonoverlapping copies wide.len() elements to the locked global memory.
        unsafe {
            let wide = to_wide(text);
            let byte_len = wide.len() * 2;

            if let Err(e) = OpenClipboard(Some(hwnd)) {
                tracing::warn!("OpenClipboard failed: {e}");
                return;
            }
            if let Err(e) = EmptyClipboard() {
                tracing::warn!("EmptyClipboard failed: {e}");
            }

            let hmem = GlobalAlloc(GMEM_MOVEABLE, byte_len)
                .inspect_err(|e| tracing::warn!("GlobalAlloc failed: {e}"))
                .ok();
            if let Some(hmem) = hmem {
                let ptr = GlobalLock(hmem) as *mut u16;
                if !ptr.is_null() {
                    std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                    let _ = GlobalUnlock(hmem);
                    if let Err(e) = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(hmem.0))) {
                        tracing::warn!("SetClipboardData failed: {e}");
                    }
                }
            }
            let _ = CloseClipboard();
        }
    }

    /// 텍스트 초기화
    fn clear_text(&self) {
        // SAFETY: source_edit and dest_edit are valid edit control handles.
        unsafe {
            Self::set_edit_text(self.source_edit, "");
            Self::set_edit_text(self.dest_edit, "");
            let _ = SetFocus(Some(self.source_edit));
        }
    }
}
