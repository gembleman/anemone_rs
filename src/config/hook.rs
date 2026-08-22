use serde::{Deserialize, Serialize};

/// 후킹 기능 설정. 기존 config.toml에 `[hook]` 블록이 없어도
/// `#[serde(default)]` 체인으로 전부 기본값(비활성)이 된다.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HookConfig {
    /// 후킹 텍스트 캡처를 화면 번역 파이프라인으로 흘려보낼지.
    /// 실제 attach는 사용자가 "후킹 대상 선택"에서 게임을 고른 순간 시작된다.
    pub enabled: bool,
    /// anemone 시작 시 마지막 대상에 자동으로 다시 붙는다.
    /// 대상 프로세스가 없으면 조용히 건너뛴다.
    pub auto_attach_last: bool,
    /// 마지막으로 붙었던 대상 pid. 0이면 없음.
    pub last_target_pid: u32,
    /// 마지막 대상 실행 파일 이름(표시용).
    #[serde(default)]
    pub last_target_name: String,
    /// 갱신형 UI 엔진(같은 문장을 계속 다시 보내는) 텍스트를 병합할 시간 창(ms).
    pub merge_window_ms: u32,
    /// 한 문장 최대 길이(문자 수). 클립보드 max_length와 같은 역할.
    pub max_text_length: u32,
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_attach_last: true,
            last_target_pid: 0,
            last_target_name: String::new(),
            merge_window_ms: 50,
            max_text_length: 2000,
        }
    }
}
