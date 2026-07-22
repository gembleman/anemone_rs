//! 사용자 지정 가능한 단축키 설정.
//!
//! config.toml에는 "Ctrl+Shift+A" 같은 사람이 읽을 수 있는 문자열로 저장하고,
//! 실제 등록 시점에 Windows 가상 키 코드/`HOT_KEY_MODIFIERS`로 변환한다.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY, VK_0, VK_1, VK_2,
    VK_3, VK_4, VK_5, VK_6, VK_7, VK_8, VK_9, VK_A, VK_B, VK_C, VK_D, VK_DOWN, VK_E, VK_F, VK_F1,
    VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11, VK_F12, VK_G, VK_H,
    VK_I, VK_J, VK_K, VK_L, VK_LEFT, VK_M, VK_N, VK_O, VK_OEM_1, VK_OEM_2, VK_OEM_3, VK_OEM_4,
    VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS, VK_P,
    VK_Q, VK_R, VK_RIGHT, VK_S, VK_SPACE, VK_T, VK_TAB, VK_U, VK_UP, VK_V, VK_W, VK_X, VK_Y, VK_Z,
};

/// 단축키 하나(수정자 조합 + 가상 키)를 나타낸다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotkeySpec {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
    pub vk: u32,
}

impl HotkeySpec {
    pub const fn new(ctrl: bool, shift: bool, alt: bool, win: bool, vk: u32) -> Self {
        Self {
            ctrl,
            shift,
            alt,
            win,
            vk,
        }
    }

    /// `RegisterHotKey`에 전달할 `HOT_KEY_MODIFIERS` 비트 조합.
    pub fn modifiers(&self) -> HOT_KEY_MODIFIERS {
        let mut modifiers = HOT_KEY_MODIFIERS(0);
        if self.ctrl {
            modifiers |= MOD_CONTROL;
        }
        if self.shift {
            modifiers |= MOD_SHIFT;
        }
        if self.alt {
            modifiers |= MOD_ALT;
        }
        if self.win {
            modifiers |= MOD_WIN;
        }
        modifiers
    }

    /// 등록 가능한 키인지 확인.
    pub fn is_valid(&self) -> bool {
        key_name_from_vk(self.vk).is_some()
    }
}

impl std::fmt::Display for HotkeySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.win {
            parts.push("Win");
        }
        let key_name = key_name_from_vk(self.vk).unwrap_or("?");
        parts.push(key_name);
        write!(f, "{}", parts.join("+"))
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HotkeyParseError {
    #[error("빈 단축키 문자열입니다")]
    Empty,
    #[error("알 수 없는 키 이름: {0}")]
    UnknownKey(String),
    #[error("키 이름이 없습니다 (수정자만 있음)")]
    MissingKey,
}

impl std::str::FromStr for HotkeySpec {
    type Err = HotkeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(HotkeyParseError::Empty);
        }

        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut win = false;
        let mut vk = None;

        for token in trimmed.split('+') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" => alt = true,
                "win" | "windows" => win = true,
                _ => {
                    vk = Some(
                        vk_from_key_name(token)
                            .ok_or_else(|| HotkeyParseError::UnknownKey(token.to_string()))?,
                    );
                }
            }
        }

        let vk = vk.ok_or(HotkeyParseError::MissingKey)?;
        Ok(HotkeySpec::new(ctrl, shift, alt, win, vk))
    }
}

impl Serialize for HotkeySpec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for HotkeySpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// 키 이름 <-> 가상 키 코드 테이블. A-Z, 0-9, 화살표, F1-F12, 자주 쓰는 OEM 키를 지원한다.
const KEY_TABLE: &[(&str, u32)] = &[
    ("A", VK_A.0 as u32),
    ("B", VK_B.0 as u32),
    ("C", VK_C.0 as u32),
    ("D", VK_D.0 as u32),
    ("E", VK_E.0 as u32),
    ("F", VK_F.0 as u32),
    ("G", VK_G.0 as u32),
    ("H", VK_H.0 as u32),
    ("I", VK_I.0 as u32),
    ("J", VK_J.0 as u32),
    ("K", VK_K.0 as u32),
    ("L", VK_L.0 as u32),
    ("M", VK_M.0 as u32),
    ("N", VK_N.0 as u32),
    ("O", VK_O.0 as u32),
    ("P", VK_P.0 as u32),
    ("Q", VK_Q.0 as u32),
    ("R", VK_R.0 as u32),
    ("S", VK_S.0 as u32),
    ("T", VK_T.0 as u32),
    ("U", VK_U.0 as u32),
    ("V", VK_V.0 as u32),
    ("W", VK_W.0 as u32),
    ("X", VK_X.0 as u32),
    ("Y", VK_Y.0 as u32),
    ("Z", VK_Z.0 as u32),
    ("0", VK_0.0 as u32),
    ("1", VK_1.0 as u32),
    ("2", VK_2.0 as u32),
    ("3", VK_3.0 as u32),
    ("4", VK_4.0 as u32),
    ("5", VK_5.0 as u32),
    ("6", VK_6.0 as u32),
    ("7", VK_7.0 as u32),
    ("8", VK_8.0 as u32),
    ("9", VK_9.0 as u32),
    ("Up", VK_UP.0 as u32),
    ("Down", VK_DOWN.0 as u32),
    ("Left", VK_LEFT.0 as u32),
    ("Right", VK_RIGHT.0 as u32),
    ("Space", VK_SPACE.0 as u32),
    ("Tab", VK_TAB.0 as u32),
    ("F1", VK_F1.0 as u32),
    ("F2", VK_F2.0 as u32),
    ("F3", VK_F3.0 as u32),
    ("F4", VK_F4.0 as u32),
    ("F5", VK_F5.0 as u32),
    ("F6", VK_F6.0 as u32),
    ("F7", VK_F7.0 as u32),
    ("F8", VK_F8.0 as u32),
    ("F9", VK_F9.0 as u32),
    ("F10", VK_F10.0 as u32),
    ("F11", VK_F11.0 as u32),
    ("F12", VK_F12.0 as u32),
    (";", VK_OEM_1.0 as u32),
    ("/", VK_OEM_2.0 as u32),
    ("`", VK_OEM_3.0 as u32),
    ("[", VK_OEM_4.0 as u32),
    ("\\", VK_OEM_5.0 as u32),
    ("]", VK_OEM_6.0 as u32),
    ("'", VK_OEM_7.0 as u32),
    (",", VK_OEM_COMMA.0 as u32),
    ("-", VK_OEM_MINUS.0 as u32),
    (".", VK_OEM_PERIOD.0 as u32),
    ("=", VK_OEM_PLUS.0 as u32),
];

fn vk_from_key_name(name: &str) -> Option<u32> {
    KEY_TABLE
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, vk)| *vk)
}

fn key_name_from_vk(vk: u32) -> Option<&'static str> {
    KEY_TABLE
        .iter()
        .find(|(_, candidate)| *candidate == vk)
        .map(|(name, _)| *name)
}

/// 등록된 4개 단축키.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HotkeyConfig {
    /// 윈도우 토글
    pub toggle_window: HotkeySpec,
    /// 텍스트 크기 증가
    pub text_size_up: HotkeySpec,
    /// 텍스트 크기 감소
    pub text_size_down: HotkeySpec,
    /// 클립보드 캡처 일시중지/재개
    pub clipboard_watch: HotkeySpec,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            toggle_window: HotkeySpec::new(true, true, false, false, VK_A.0 as u32),
            text_size_up: HotkeySpec::new(true, true, false, false, VK_UP.0 as u32),
            text_size_down: HotkeySpec::new(true, true, false, false, VK_DOWN.0 as u32),
            clipboard_watch: HotkeySpec::new(true, true, false, false, VK_C.0 as u32),
        }
    }
}

impl HotkeyConfig {
    /// (설명, HotkeySpec) 4개를 고정 순서로 순회한다. UI/등록 로직이 공유하는 단일 소스.
    pub fn entries(&self) -> [(HotkeySlot, HotkeySpec); 4] {
        [
            (HotkeySlot::ToggleWindow, self.toggle_window),
            (HotkeySlot::TextSizeUp, self.text_size_up),
            (HotkeySlot::TextSizeDown, self.text_size_down),
            (HotkeySlot::ClipboardWatch, self.clipboard_watch),
        ]
    }

    pub fn get(&self, slot: HotkeySlot) -> HotkeySpec {
        match slot {
            HotkeySlot::ToggleWindow => self.toggle_window,
            HotkeySlot::TextSizeUp => self.text_size_up,
            HotkeySlot::TextSizeDown => self.text_size_down,
            HotkeySlot::ClipboardWatch => self.clipboard_watch,
        }
    }

    pub fn set(&mut self, slot: HotkeySlot, spec: HotkeySpec) {
        match slot {
            HotkeySlot::ToggleWindow => self.toggle_window = spec,
            HotkeySlot::TextSizeUp => self.text_size_up = spec,
            HotkeySlot::TextSizeDown => self.text_size_down = spec,
            HotkeySlot::ClipboardWatch => self.clipboard_watch = spec,
        }
    }

    /// 같은 수정자+키 조합이 서로 다른 슬롯에 중복 배정되어 있는지 확인한다.
    /// 충돌이 있으면 먼저 발견된 (slot_a, slot_b) 쌍을 반환한다.
    pub fn find_conflict(&self) -> Option<(HotkeySlot, HotkeySlot)> {
        let entries = self.entries();
        for i in 0..entries.len() {
            for j in (i + 1)..entries.len() {
                if entries[i].1 == entries[j].1 {
                    return Some((entries[i].0, entries[j].0));
                }
            }
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeySlot {
    ToggleWindow,
    TextSizeUp,
    TextSizeDown,
    ClipboardWatch,
}

impl HotkeySlot {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ToggleWindow => "윈도우 토글",
            Self::TextSizeUp => "텍스트 크기 증가",
            Self::TextSizeDown => "텍스트 크기 감소",
            Self::ClipboardWatch => "클립보드 캡처 일시중지/재개",
        }
    }
}

// VIRTUAL_KEY 캐스팅 확인용 (u32 <-> VIRTUAL_KEY 변환은 hotkey.rs와 일관되게 u32로 통일한다).
const _: fn(VIRTUAL_KEY) -> u32 = |vk| vk.0 as u32;

#[cfg(test)]
#[path = "../../tests/unit/config/hotkey.rs"]
mod tests;
