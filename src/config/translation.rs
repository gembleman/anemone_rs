use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::LlmConfig;

/// 번역 설정
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranslationConfig {
    /// 번역 엔진: "eztrans", "google", "deepl", "papago", "llm"
    #[serde(default = "default_engine", deserialize_with = "deserialize_engine")]
    pub engine: String,
    /// 소스 언어 (ISO 639-1 코드): "ja", "ko", "en", "zh", etc.
    #[serde(
        default = "default_source_lang",
        deserialize_with = "deserialize_language"
    )]
    pub source_lang: String,
    /// 타겟 언어 (ISO 639-1 코드): "ja", "ko", "en", "zh", etc.
    #[serde(
        default = "default_target_lang",
        deserialize_with = "deserialize_language"
    )]
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
    /// Ncloud Papago Application Client ID
    #[serde(default)]
    pub papago_client_id: String,
    /// Ncloud Papago Application Client Secret
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
    pub fn get_engine(
        &self,
    ) -> Result<crate::translation::TranslationEngine, crate::translation::EnumParseError> {
        self.engine.parse()
    }

    /// 소스 언어를 번역용 언어 태그로 가져오기
    pub fn get_source_language(&self) -> Result<crate::translation::Language, InvalidLanguageCode> {
        crate::translation::lang_utils::from_code(&self.source_lang)
            .ok_or_else(|| InvalidLanguageCode(self.source_lang.clone()))
    }

    /// 타겟 언어를 번역용 언어 태그로 가져오기
    pub fn get_target_language(&self) -> Result<crate::translation::Language, InvalidLanguageCode> {
        crate::translation::lang_utils::from_code(&self.target_lang)
            .ok_or_else(|| InvalidLanguageCode(self.target_lang.clone()))
    }

    /// 엔진 설정
    pub fn set_engine(&mut self, engine: crate::translation::TranslationEngine) {
        self.engine = engine.to_str().to_string();
    }

    /// 소스 언어 설정
    pub fn set_source_language(&mut self, lang: crate::translation::Language) {
        self.source_lang = crate::translation::lang_utils::to_code(lang).to_string();
    }

    /// 타겟 언어 설정
    pub fn set_target_language(&mut self, lang: crate::translation::Language) {
        self.target_lang = crate::translation::lang_utils::to_code(lang).to_string();
    }

    // UI 호환 API.

    /// 엔진 문자열을 u8로 변환 (UI 호환용)
    pub fn engine_as_u8(&self) -> Result<u8, crate::translation::EnumParseError> {
        self.get_engine().map(|engine| engine as u8)
    }

    /// 언어 인덱스를 가져오기 (UI 콤보박스용)
    pub fn source_lang_index(
        &self,
        engine: crate::translation::TranslationEngine,
    ) -> Result<usize, String> {
        let lang = self
            .get_source_language()
            .map_err(|error| error.to_string())?;
        let supported = engine.supported_source_languages();
        supported.iter().position(|&l| l == lang).ok_or_else(|| {
            format!(
                "{} 엔진이 소스 언어 {}를 지원하지 않습니다",
                engine.to_str(),
                self.source_lang
            )
        })
    }

    /// DeepL 멀티 키 전략
    pub fn deepl_strategy(&self) -> crate::translation::worker::DeepLStrategy {
        use crate::translation::worker::DeepLStrategy;
        match self.deepl_strategy.to_lowercase().as_str() {
            "round-robin" | "roundrobin" | "rr" => DeepLStrategy::RoundRobin,
            _ => DeepLStrategy::Failover,
        }
    }

    /// 빈 값을 뺀 DeepL key 목록. 보조 key가 있으면 legacy 단일 key보다 우선한다.
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

fn deserialize_engine<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = String::deserialize(deserializer)?;
    value
        .parse::<crate::translation::TranslationEngine>()
        .map_err(D::Error::custom)?;
    Ok(value)
}

fn deserialize_language<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = String::deserialize(deserializer)?;
    crate::translation::lang_utils::from_code(&value)
        .ok_or_else(|| D::Error::custom(format!("알 수 없는 번역 언어 코드: {value}")))?;
    Ok(value)
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("알 수 없는 번역 언어 코드: {0}")]
pub struct InvalidLanguageCode(pub String);

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
