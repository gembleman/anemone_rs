use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{CustomApiConfig, LlmConfig};

/// EzTrans 번역 결과에 적용하는 후처리 치환 한 항목.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EzTransPostprocessEntry {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub target: String,
}

/// 번역 설정
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranslationConfig {
    /// 번역 엔진: "eztrans", "google", "deepl", "papago", "llm", "custom"
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
    /// EzTrans 평면 사전 경로. 이전 설정 키 이름은 serde 별칭으로 호환한다.
    #[serde(default, alias = "eztrans_dll_path")]
    pub eztrans_dictionary_path: String,
    /// EzTrans Ehnd 필터/사용자 사전 경로.
    /// 이전 `eztrans_dat_path`는 역직렬화 시 형제 `Ehnd` 경로로 변환한다.
    #[serde(
        default,
        alias = "eztrans_dat_path",
        deserialize_with = "deserialize_eztrans_ehnd_path"
    )]
    pub eztrans_ehnd_path: String,
    /// 파일 번역에서 동시에 유지할 EzTrans helper 프로세스 수
    #[serde(default = "default_eztrans_process_count")]
    pub eztrans_process_count: u32,
    /// EzTrans가 반환한 번역문에만 적용하는 후처리 사전.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub eztrans_postprocess_dictionary: Vec<EzTransPostprocessEntry>,
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
    /// 이전 버전의 단일 사용자 정의 API 설정. 로드 시 `custom_apis`로 마이그레이션된다.
    #[serde(default, skip_serializing_if = "CustomApiConfig::is_default")]
    pub custom: CustomApiConfig,
    /// 현재 선택된 사용자 정의 API 이름. 비어 있으면 첫 항목을 사용한다.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub custom_api: String,
    /// 이름을 가진 사용자 정의 JSON REST API 설정 목록.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_apis: Vec<CustomApiConfig>,
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

fn default_eztrans_process_count() -> u32 {
    2
}

fn deserialize_eztrans_ehnd_path<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let path = std::path::Path::new(&value);
    if path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("Dat"))
    {
        return Ok(path
            .parent()
            .map(|parent| parent.join("Ehnd"))
            .unwrap_or_else(|| PathBuf::from("Ehnd"))
            .to_string_lossy()
            .into_owned());
    }
    Ok(value)
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

    /// 현재 선택된 Custom API를 반환한다. 새 목록이 없으면 이전 단일 설정을 사용한다.
    pub fn active_custom_api(&self) -> Result<&CustomApiConfig, CustomApiSelectionError> {
        if self.custom_apis.is_empty() {
            return Ok(&self.custom);
        }

        self.validate_custom_api_names()?;
        if self.custom_api.is_empty() {
            return Ok(&self.custom_apis[0]);
        }
        self.custom_apis
            .iter()
            .find(|api| api.name == self.custom_api)
            .ok_or_else(|| CustomApiSelectionError::NotFound(self.custom_api.clone()))
    }

    /// 현재 선택된 Custom API를 변경 가능하게 반환한다.
    pub fn active_custom_api_mut(
        &mut self,
    ) -> Result<&mut CustomApiConfig, CustomApiSelectionError> {
        if self.custom_apis.is_empty() {
            return Ok(&mut self.custom);
        }

        self.validate_custom_api_names()?;
        let index = if self.custom_api.is_empty() {
            0
        } else {
            self.custom_apis
                .iter()
                .position(|api| api.name == self.custom_api)
                .ok_or_else(|| CustomApiSelectionError::NotFound(self.custom_api.clone()))?
        };
        Ok(&mut self.custom_apis[index])
    }

    pub fn active_custom_api_index(&self) -> Result<usize, CustomApiSelectionError> {
        if self.custom_apis.is_empty() {
            return Ok(0);
        }
        self.validate_custom_api_names()?;
        if self.custom_api.is_empty() {
            return Ok(0);
        }
        self.custom_apis
            .iter()
            .position(|api| api.name == self.custom_api)
            .ok_or_else(|| CustomApiSelectionError::NotFound(self.custom_api.clone()))
    }

    pub fn select_custom_api(&mut self, name: &str) -> Result<bool, CustomApiSelectionError> {
        if self.custom_apis.is_empty() {
            if name == self.custom.name {
                return Ok(false);
            }
            return Err(CustomApiSelectionError::NotFound(name.to_string()));
        }
        self.validate_custom_api_names()?;
        if !self.custom_apis.iter().any(|api| api.name == name) {
            return Err(CustomApiSelectionError::NotFound(name.to_string()));
        }
        if self.custom_api == name {
            return Ok(false);
        }
        self.custom_api = name.to_string();
        Ok(true)
    }

    fn validate_custom_api_names(&self) -> Result<(), CustomApiSelectionError> {
        let mut names = std::collections::HashSet::new();
        for api in &self.custom_apis {
            if api.name.trim().is_empty() {
                return Err(CustomApiSelectionError::EmptyName);
            }
            if !names.insert(api.name.as_str()) {
                return Err(CustomApiSelectionError::DuplicateName(api.name.clone()));
            }
        }
        Ok(())
    }

    /// v0의 단일 Custom API 설정을 v1의 이름 기반 목록으로 옮긴다.
    pub(crate) fn migrate_legacy_custom_api(&mut self) {
        if self.custom_apis.is_empty() && !self.custom.is_default() {
            let legacy = std::mem::take(&mut self.custom);
            self.custom_api = legacy.name.clone();
            self.custom_apis.push(legacy);
        } else if !self.custom_apis.is_empty() {
            self.custom = CustomApiConfig::default();
            if self.custom_api.is_empty() {
                self.custom_api = self.custom_apis[0].name.clone();
            }
        }
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
    pub(crate) fn deepl_strategy(&self) -> crate::translation::DeepLStrategy {
        use crate::translation::DeepLStrategy;
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
            eztrans_dictionary_path: default_eztrans_subpath("JisJK.flat.bin"),
            eztrans_ehnd_path: default_eztrans_subpath("Ehnd"),
            eztrans_process_count: default_eztrans_process_count(),
            eztrans_postprocess_dictionary: Vec::new(),
            deepl_api_key: String::new(),
            deepl_keys: Vec::new(),
            deepl_strategy: default_deepl_strategy(),
            papago_client_id: String::new(),
            papago_client_secret: String::new(),
            llm: LlmConfig::default(),
            custom: CustomApiConfig::default(),
            custom_api: String::new(),
            custom_apis: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CustomApiSelectionError {
    #[error("Custom API 이름은 비워 둘 수 없습니다.")]
    EmptyName,
    #[error("Custom API 이름이 중복되었습니다: {0}")]
    DuplicateName(String),
    #[error("선택한 Custom API를 찾을 수 없습니다: {0}")]
    NotFound(String),
}
