//! Engine과 언어를 선택해 수동 번역하는 dialog.

mod combo;
mod controls;
mod flow;

use std::rc::Rc;

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{EN_CHANGE, IDCANCEL, KillTimer, WM_COMMAND, WM_TIMER},
};

use super::host::{DialogHost, DialogResult, HostedDialog};
use crate::app::action::AppActionSender;

use crate::app::messages::WM_TRANSLATION_COMPLETE;
use crate::app::services::GuiTranslationHost;
use crate::config::Config;
use crate::translation::manual::ManualTranslationOptions;
type Result<T> = windows_core::Result<T>;

// 컨트롤 ID
mod ctrl_id {
    pub const DIALOG: u16 = 105;
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
    pub const LLM_GROUP: u16 = 2040;
    pub const LLM_PROVIDER_LABEL: u16 = 2041;
    pub const LLM_MODEL_LABEL: u16 = 2042;
    pub const LLM_API_KEY_LABEL: u16 = 2043;
}

// 서브클래스 ID (uIdSubclass): 컨트롤별로 구분
mod subclass_id {
    pub const SOURCE_EDIT: usize = 1;
    pub const DEST_EDIT: usize = 2;
}

const AUTO_TRANSLATE_TIMER: usize = 0xA710;

/// 번역 대화상자
pub struct TranslateDialog {
    hwnd: HWND,
    config: Config,
    translation_service: Rc<GuiTranslationHost>,
    applied_dpi: u32,
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
    manual_options: ManualTranslationOptions,
    /// 현재 최신 요청 ID. 새 자동 요청이 들어오면 워커가 이전 요청을 취소한다.
    in_flight_id: Option<u64>,
    last_submitted_source: String,
    actions: AppActionSender,
    session: u64,
}

pub(crate) struct TranslateInit {
    config: Config,
    translation_service: Rc<GuiTranslationHost>,
    actions: AppActionSender,
    session: u64,
}

impl HostedDialog for TranslateDialog {
    type Init = TranslateInit;
    const RESOURCE_ID: u16 = ctrl_id::DIALOG;

    fn create(hwnd: HWND, init: Self::Init) -> Result<Self> {
        let mut dialog = Self::new(
            hwnd,
            init.config,
            init.translation_service,
            init.actions,
            init.session,
        );
        dialog.initialize_controls()?;
        Ok(dialog)
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> DialogResult {
        match msg {
            WM_TRANSLATION_COMPLETE => {
                self.handle_translation_complete();
                DialogResult::Handled(1)
            }
            WM_COMMAND => {
                let id = (wparam & 0xFFFF) as u16;
                if id == IDCANCEL as u16 {
                    return DialogResult::Close(1);
                }
                let notify_code = ((wparam >> 16) & 0xFFFF) as u32;
                if id == ctrl_id::SOURCE_EDIT && notify_code == EN_CHANGE {
                    self.invalidate_stale_translation();
                    if self.one_go {
                        self.schedule_auto_translate();
                    }
                }
                self.handle_command(id, notify_code);
                DialogResult::Handled(1)
            }
            WM_TIMER if wparam == AUTO_TRANSLATE_TIMER => {
                // SAFETY: self.hwnd는 살아 있는 번역 창이다.
                unsafe {
                    let _ = KillTimer(self.hwnd, AUTO_TRANSLATE_TIMER);
                }
                self.do_translate();
                DialogResult::Handled(1)
            }
            _ => DialogResult::Unhandled,
        }
    }

    fn applied_dpi(&mut self) -> Option<&mut u32> {
        Some(&mut self.applied_dpi)
    }

    fn destroy(&mut self) {
        // SAFETY: self.hwnd는 아직 파괴 중인 유효한 창이다.
        unsafe {
            let _ = KillTimer(self.hwnd, AUTO_TRANSLATE_TIMER);
        }
        self.translation_service.unregister(self.hwnd);
        self.actions.translate_dialog_closed(self.session);
    }

    fn can_defer(msg: u32) -> bool {
        matches!(msg, WM_COMMAND | WM_TRANSLATION_COMPLETE)
    }
}

impl TranslateDialog {
    fn new(
        hwnd: HWND,
        mut config: Config,
        translation_service: Rc<GuiTranslationHost>,
        actions: AppActionSender,
        session: u64,
    ) -> Self {
        config
            .translation
            .activate_route(crate::config::TranslationRoute::Manual);
        Self {
            hwnd,
            config,
            translation_service,
            applied_dpi: crate::dpi::dpi_for_window(hwnd),
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
            manual_options: ManualTranslationOptions::default(),
            in_flight_id: None,
            last_submitted_source: String::new(),
            actions,
            session,
        }
    }

    /// `resources/translate.rc`의 모델리스 DIALOGEX 리소스를 연다.
    pub fn show(
        parent: HWND,
        config: Config,
        translation_service: Rc<GuiTranslationHost>,
        actions: AppActionSender,
        session: u64,
    ) -> Result<HWND> {
        DialogHost::<Self>::show(
            parent,
            TranslateInit {
                config,
                translation_service,
                actions,
                session,
            },
        )
    }

    pub(crate) fn current_session() -> Option<u64> {
        DialogHost::<Self>::with_state(|dialog| dialog.session)
    }
}
