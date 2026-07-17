use serde::{Deserialize, Serialize};

/// 후크 설정
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookConfig {
    /// 활성화된 후크 목록
    pub active_hooks: Vec<String>,
    /// 비활성화된 후크 목록
    pub inactive_hooks: Vec<String>,
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            active_hooks: vec![
                "클립보드".to_string(),
                "자동저장".to_string(),
                "알림".to_string(),
            ],
            inactive_hooks: vec!["로그".to_string(), "번역기록".to_string()],
        }
    }
}
