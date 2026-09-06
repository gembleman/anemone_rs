//! 전역 단축키 ID → 명령 매핑과 단축키 재등록.

use super::Result;
use crate::app::{App, state};
use crate::config::HotkeySlot;
use crate::hotkey::HotkeyManager;

pub(crate) fn command_from_hotkey_id(id: i32) -> Option<state::AppCommand> {
    match HotkeyManager::slot_for_id(id)? {
        HotkeySlot::ToggleWindow => Some(state::AppCommand::WindowShow),
        HotkeySlot::TextSizeUp => Some(state::AppCommand::TextSizeUp),
        HotkeySlot::TextSizeDown => Some(state::AppCommand::TextSizeDown),
        HotkeySlot::ClipboardWatch => Some(state::AppCommand::ClipboardWatch),
    }
}

impl App {
    pub(in crate::app) fn handle_hotkey(&mut self, id: i32) -> Result<()> {
        if let Some(command) = command_from_hotkey_id(id) {
            let effects = self.model.update(state::AppAction::Command(command));
            self.run_effects(effects);
        }
        Ok(())
    }

    /// 설정 다이얼로그에서 단축키가 바뀐 뒤 호출된다. 기존 등록을 모두 해제하고
    /// 현재 config 기준으로 다시 등록해 실행 중에도 즉시 새 단축키가 반영되게 한다.
    pub(in crate::app) fn reregister_hotkeys(&mut self) {
        let Some(hotkey) = self.hotkey.as_mut() else {
            return;
        };
        hotkey.unregister_all();
        if let Err(error) = hotkey.register_from_config(&self.model.config.hotkeys) {
            tracing::warn!("단축키를 다시 등록하지 못했습니다: {error}");
            crate::dialogs::helpers::show_error_message(
                self.hwnd,
                "단축키 등록 오류",
                &format!(
                    "단축키를 등록하지 못했습니다. 다른 프로그램이 이미 사용 중일 수 있습니다.\n\n{error}"
                ),
            );
        }
    }
}
