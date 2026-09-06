use serde::{Deserialize, Serialize};

const MAX_SAVED_PROFILES: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedHookProfile {
    pub process_name: String,
    pub exe_sha256: Option<String>,
    pub hook_name: String,
    pub hook_code: Option<String>,
}

/// 후킹 기능 설정. 기존 config.toml에 `[hook]` 블록이 없어도
/// `#[serde(default)]` 체인으로 전부 기본값(비활성)이 된다.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HookConfig {
    /// 후킹 텍스트 캡처를 화면 번역 파이프라인으로 흘려보낼지.
    /// 실제 attach는 사용자가 "후킹 관리"에서 게임을 고른 순간 시작된다.
    pub enabled: bool,
    /// anemone 시작 시 마지막 대상에 자동으로 다시 붙는다.
    /// 대상 프로세스가 없으면 조용히 건너뛴다.
    pub auto_attach_last: bool,
    /// 마지막으로 붙었던 대상 pid. 0이면 없음.
    pub last_target_pid: u32,
    /// 마지막 대상 실행 파일 이름(표시용).
    #[serde(default)]
    pub last_target_name: String,
    /// 후킹 텍스트를 한 문장으로 모을 시간 창(ms). 마지막 조각 이후 이만큼
    /// 조용하면 문장으로 낸다. 원본 LunaHost `TextThread::flushDelay`가 100이다.
    /// 짧게 잡으면 글자 단위 훅이 문장 한가운데서 끊긴다 — KiriKiri 게임에서
    /// 글자 간격이 80~90ms까지 벌어지는 것을 실기로 확인했다.
    pub merge_window_ms: u32,
    /// 한 문장 최대 길이(문자 수). 클립보드 max_length와 같은 역할.
    pub max_text_length: u32,
    /// 후킹 디버그 로그(`logs/lunahook.log`)를 남긴다. 주입·핸드셰이크·후크
    /// 설치와 각 텍스트 스레드의 원문이 들어간다. 앱 로그는 원문을 남기지
    /// 않으므로 파일을 따로 쓴다.
    ///
    /// 창에서 바꾸지 않는다. `config.toml`을 고치고 다시 시작해야 한다 — 로그
    /// subscriber는 프로세스당 한 번만 세운다.
    #[serde(default)]
    pub debug_log: bool,
    /// 진단용 토큰. `debug_log`와 함께 개발 빌드의 진단 경로에서만 읽는다.
    /// 창에서 바꾸지 않는다.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub diag_token: String,
    /// 게임별로 마지막에 사용자가 선택한 텍스트 후크.
    #[serde(default)]
    pub saved_profiles: Vec<SavedHookProfile>,
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_attach_last: true,
            last_target_pid: 0,
            last_target_name: String::new(),
            merge_window_ms: 100,
            max_text_length: 2000,
            debug_log: false,
            diag_token: String::new(),
            saved_profiles: Vec::new(),
        }
    }
}

/// 병합 창의 하한(ms). 원본 LunaHost `flushDelay`와 같다. 종전 기본값 50은
/// 글자 단위 훅에서 문장을 한가운데서 끊었다 — KiriKiri 게임의 글자 간격이
/// 80~90ms까지 벌어지는 것을 실기로 확인했다. 종전 설정 파일도 함께 올린다.
pub(crate) const MIN_MERGE_WINDOW_MS: u32 = 100;

/// 병합 창의 상한(ms). 원본은 상한을 두지 않지만, 여기서는 사용자가 창에서
/// 직접 값을 넣는다. 창이 지나야 문장이 나가므로 큰 값은 출력이 멈춘 것처럼
/// 보인다.
pub(crate) const MAX_MERGE_WINDOW_MS: u32 = 5_000;

impl HookConfig {
    pub(crate) fn normalize(&mut self) {
        self.merge_window_ms = self
            .merge_window_ms
            .clamp(MIN_MERGE_WINDOW_MS, MAX_MERGE_WINDOW_MS);
        self.saved_profiles.retain(|profile| {
            (!profile.process_name.is_empty() || profile.exe_sha256.is_some())
                && profile.process_name.chars().count() <= 260
                && profile.hook_name.chars().count() <= 128
                && profile
                    .hook_code
                    .as_ref()
                    .is_none_or(|code| code.chars().count() <= 2_048)
        });
        if self.saved_profiles.len() > MAX_SAVED_PROFILES {
            let excess = self.saved_profiles.len() - MAX_SAVED_PROFILES;
            self.saved_profiles.drain(..excess);
        }
    }

    pub fn saved_profile(
        &self,
        process_name: &str,
        exe_sha256: Option<&str>,
    ) -> Option<&SavedHookProfile> {
        if let Some(digest) = exe_sha256 {
            self.saved_profiles
                .iter()
                .find(|profile| profile.exe_sha256.as_deref() == Some(digest))
                .or_else(|| {
                    self.saved_profiles.iter().find(|profile| {
                        profile.exe_sha256.is_none()
                            && profile.process_name.eq_ignore_ascii_case(process_name)
                    })
                })
        } else {
            self.saved_profiles
                .iter()
                .find(|profile| profile.process_name.eq_ignore_ascii_case(process_name))
        }
    }

    pub fn save_profile(&mut self, profile: SavedHookProfile) {
        self.saved_profiles.retain(|saved| {
            let same_hash = profile.exe_sha256.is_some()
                && saved.exe_sha256.as_deref() == profile.exe_sha256.as_deref();
            !same_hash
                && !saved
                    .process_name
                    .eq_ignore_ascii_case(&profile.process_name)
        });
        self.saved_profiles.push(profile);
        if self.saved_profiles.len() > MAX_SAVED_PROFILES {
            let excess = self.saved_profiles.len() - MAX_SAVED_PROFILES;
            self.saved_profiles.drain(..excess);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/config/hook.rs"]
mod tests;
