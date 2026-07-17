use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::LlmConfig;

/// 번역 설정
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranslationConfig {
    /// 번역 엔진: "eztrans", "google", "deepl", "papago", "llm"
    #[serde(default = "default_engine")]
    pub engine: String,
    /// 소스 언어 (ISO 639-1 코드): "ja", "ko", "en", "zh", etc.
    #[serde(default = "default_source_lang")]
    pub source_lang: String,
    /// 타겟 언어 (ISO 639-1 코드): "ja", "ko", "en", "zh", etc.
    #[serde(default = "default_target_lang")]
    pub target_lang: String,
    /// EzTrans DLL 경로
    #[serde(default)]
    pub eztrans_dll_path: String,
    /// EzTrans Dat 경로
    #[serde(default)]
    pub eztrans_dat_path: String,
    /// DeepL API 키 (단일 키 — 하위 호환). `deepl_keys`가 비어 있을 때만 사용.
    #[serde(default)]
    pub deepl_api_key: String,
    /// DeepL 보조 API 키 목록 (멀티 키 폴백). 비어 있지 않으면 `deepl_api_key`보다 우선.
    #[serde(default)]
    pub deepl_keys: Vec<String>,
    /// DeepL 다중 키 전략: "failover" | "round-robin"
    #[serde(default = "default_deepl_strategy")]
    pub deepl_strategy: String,
    /// Papago Naver Client ID
    #[serde(default)]
    pub papago_client_id: String,
    /// Papago Naver Client Secret
    #[serde(default)]
    pub papago_client_secret: String,
    /// LLM 설정
    #[serde(default)]
    pub llm: LlmConfig,
}

fn default_engine() -> String {
    "eztrans".to_string()
}

fn default_source_lang() -> String {
    "ja".to_string()
}

fn default_target_lang() -> String {
    "ko".to_string()
}

fn default_deepl_strategy() -> String {
    "failover".to_string()
}

impl TranslationConfig {
    /// 엔진 문자열로 가져오기
    pub fn get_engine(&self) -> crate::translation::TranslationEngine {
        crate::translation::TranslationEngine::from_str(&self.engine)
    }

    /// 소스 언어를 isolang::Language로 가져오기
    pub fn get_source_language(&self) -> isolang::Language {
        crate::translation::lang_utils::from_code(&self.source_lang)
            .unwrap_or(isolang::Language::Jpn)
    }

    /// 타겟 언어를 isolang::Language로 가져오기
    pub fn get_target_language(&self) -> isolang::Language {
        crate::translation::lang_utils::from_code(&self.target_lang)
            .unwrap_or(isolang::Language::Kor)
    }

    /// 엔진 설정
    pub fn set_engine(&mut self, engine: crate::translation::TranslationEngine) {
        self.engine = engine.to_str().to_string();
    }

    /// 소스 언어 설정
    pub fn set_source_language(&mut self, lang: isolang::Language) {
        self.source_lang = crate::translation::lang_utils::to_code(lang).to_string();
    }

    /// 타겟 언어 설정
    pub fn set_target_language(&mut self, lang: isolang::Language) {
        self.target_lang = crate::translation::lang_utils::to_code(lang).to_string();
    }

    // ========== 하위 호환용 메서드들 (UI에서 사용) ==========

    /// 엔진 문자열을 u8로 변환 (UI 호환용)
    pub fn engine_as_u8(&self) -> u8 {
        match self.engine.to_lowercase().as_str() {
            "eztrans" => 0,
            "google" => 1,
            "deepl" => 2,
            "papago" => 3,
            "llm" => 4,
            _ => 0,
        }
    }

    /// 언어 인덱스를 가져오기 (UI 콤보박스용)
    pub fn source_lang_index(&self, engine: crate::translation::TranslationEngine) -> usize {
        let lang = self.get_source_language();
        let supported = engine.supported_source_languages();
        supported.iter().position(|&l| l == lang).unwrap_or(0)
    }

    /// 언어 인덱스를 가져오기 (UI 콤보박스용)
    pub fn target_lang_index(&self, engine: crate::translation::TranslationEngine) -> usize {
        let lang = self.get_target_language();
        let supported = engine.supported_target_languages();
        supported.iter().position(|&l| l == lang).unwrap_or(0)
    }

    /// 인덱스로 소스 언어 설정 (UI 콤보박스용)
    pub fn set_source_lang_by_index(
        &mut self,
        index: usize,
        engine: crate::translation::TranslationEngine,
    ) {
        let supported = engine.supported_source_languages();
        if let Some(&lang) = supported.get(index) {
            self.set_source_language(lang);
        }
    }

    /// 인덱스로 타겟 언어 설정 (UI 콤보박스용)
    pub fn set_target_lang_by_index(
        &mut self,
        index: usize,
        engine: crate::translation::TranslationEngine,
    ) {
        let supported = engine.supported_target_languages();
        if let Some(&lang) = supported.get(index) {
            self.set_target_language(lang);
        }
    }

    /// DeepL 멀티 키 전략
    pub fn deepl_strategy(&self) -> crate::translation::worker::DeepLStrategy {
        use crate::translation::worker::DeepLStrategy;
        match self.deepl_strategy.to_lowercase().as_str() {
            "round-robin" | "roundrobin" | "rr" => DeepLStrategy::RoundRobin,
            _ => DeepLStrategy::Failover,
        }
    }

    /// 워커로 넘길 DeepL 키 목록 (보조 키가 있으면 그것을, 없으면 단일 키만)
    ///
    /// 빈 키는 제거해서 워커가 건너뛸 필요가 없게 한다.
    pub fn deepl_effective_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = if !self.deepl_keys.is_empty() {
            self.deepl_keys
                .iter()
                .filter(|k| !k.is_empty())
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        if keys.is_empty() && !self.deepl_api_key.is_empty() {
            keys.push(self.deepl_api_key.clone());
        }
        keys
    }
}

/// 실행 파일 옆 `eztrans_dll/` 내부 경로를 문자열로 반환한다.
/// 실행 경로 조회에 실패하면 상대 경로(`eztrans_dll/<sub>`)로 폴백한다.
fn default_eztrans_subpath(sub: &str) -> String {
    let rel = PathBuf::from("eztrans_dll").join(sub);
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        return exe_dir.join(&rel).to_string_lossy().into_owned();
    }
    rel.to_string_lossy().into_owned()
}

impl Default for TranslationConfig {
    fn default() -> Self {
        Self {
            engine: "eztrans".to_string(),
            source_lang: "ja".to_string(),
            target_lang: "ko".to_string(),
            eztrans_dll_path: default_eztrans_subpath("J2KEngine.dll"),
            eztrans_dat_path: default_eztrans_subpath("Dat"),
            deepl_api_key: String::new(),
            deepl_keys: Vec::new(),
            deepl_strategy: default_deepl_strategy(),
            papago_client_id: String::new(),
            papago_client_secret: String::new(),
            llm: LlmConfig::default(),
        }
    }
}
