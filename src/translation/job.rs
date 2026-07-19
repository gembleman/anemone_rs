//! 호출 환경과 무관한 검증된 번역 작업 구성.

use std::sync::Arc;

use crate::config::TranslationConfig;
use crate::translation::custom::CustomApiCallParams;
use crate::translation::llm::LlmCallParams;
use crate::translation::{
    DeepLApiTier, EzTransProcessConfig, Language, TranslationEngine, prepare_eztrans,
};

/// DeepL 다중 키 폴백 전략.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DeepLStrategy {
    #[default]
    Failover,
    RoundRobin,
}

/// 번역 언어쌍. 엔진 지원 여부는 [`PreparedJob`] 생성 시 검증한다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LanguagePair {
    source: Language,
    target: Language,
}

impl LanguagePair {
    pub const fn new(source: Language, target: Language) -> Self {
        Self { source, target }
    }

    pub const fn source(self) -> Language {
        self.source
    }

    pub const fn target(self) -> Language {
        self.target
    }
}

/// 엔진과 인증 정보를 하나의 variant로 결합한 내부 실행 backend.
pub(crate) enum PreparedEngineKind {
    EzTrans(EzTransProcessConfig),
    Google,
    DeepL {
        keys: Vec<(String, DeepLApiTier)>,
        strategy: DeepLStrategy,
    },
    Papago {
        client_id: String,
        client_secret: String,
    },
    Llm(LlmCallParams),
    Custom(CustomApiCallParams),
}

impl PreparedEngineKind {
    fn engine(&self) -> TranslationEngine {
        match self {
            Self::EzTrans(_) => TranslationEngine::EzTrans,
            Self::Google => TranslationEngine::Google,
            Self::DeepL { .. } => TranslationEngine::DeepL,
            Self::Papago { .. } => TranslationEngine::Papago,
            Self::Llm(_) => TranslationEngine::Llm,
            Self::Custom(_) => TranslationEngine::Custom,
        }
    }
}

/// 비밀값을 복제하지 않고 공유하는 검증된 번역 엔진.
#[derive(Clone)]
pub struct PreparedEngine(Arc<PreparedEngineKind>);

impl PreparedEngine {
    pub fn engine(&self) -> TranslationEngine {
        self.0.engine()
    }

    pub fn display_name(&self) -> &'static str {
        self.engine().to_str()
    }

    pub fn max_input_chars(&self) -> usize {
        self.engine().max_input_chars()
    }

    pub fn is_blocking(&self) -> bool {
        matches!(self.0.as_ref(), PreparedEngineKind::EzTrans(_))
    }

    pub fn supports_batch(&self) -> bool {
        matches!(self.0.as_ref(), PreparedEngineKind::EzTrans(_))
    }

    pub(crate) fn kind(&self) -> &PreparedEngineKind {
        self.0.as_ref()
    }

    pub(crate) fn eztrans_process(&self) -> Option<&EzTransProcessConfig> {
        match self.0.as_ref() {
            PreparedEngineKind::EzTrans(config) => Some(config),
            _ => None,
        }
    }
}

impl std::fmt::Debug for PreparedEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedEngine")
            .field("engine", &self.engine())
            .finish_non_exhaustive()
    }
}

/// GUI, CLI와 파일 번역이 공유하는 검증된 실행 작업.
///
/// `Debug`는 엔진과 언어만 표시하고 자격증명은 노출하지 않는다.
#[derive(Clone)]
pub struct PreparedJob {
    engine: PreparedEngine,
    languages: LanguagePair,
}

impl PreparedJob {
    pub fn google(source: Language, target: Language) -> Result<Self, TranslationConfigError> {
        Self::from_kind(PreparedEngineKind::Google, source, target)
    }

    pub fn eztrans(
        dll_path: String,
        dat_path: String,
        process_count: usize,
        source: Language,
        target: Language,
    ) -> Result<Self, TranslationConfigError> {
        if dll_path.trim().is_empty() || dat_path.trim().is_empty() {
            return Err(TranslationConfigError::MissingEzTransPath);
        }
        Self::from_kind(
            PreparedEngineKind::EzTrans(EzTransProcessConfig {
                dll_path,
                dat_path,
                process_count: crate::config::limits::eztrans_process_count_usize(process_count),
            }),
            source,
            target,
        )
    }

    pub fn from_config(config: &TranslationConfig) -> Result<Self, TranslationConfigError> {
        let engine = config
            .get_engine()
            .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
        let source = config
            .get_source_language()
            .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
        let target = config
            .get_target_language()
            .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
        Self::with_engine_languages(config, engine, source, target)
    }

    /// CLI 등의 명시적 엔진/언어 재정의도 동일한 backend 구성 규칙을 사용한다.
    pub fn with_engine_languages(
        config: &TranslationConfig,
        engine: TranslationEngine,
        source: Language,
        target: Language,
    ) -> Result<Self, TranslationConfigError> {
        let kind =
            match engine {
                TranslationEngine::EzTrans => {
                    if config.eztrans_dll_path.trim().is_empty()
                        || config.eztrans_dat_path.trim().is_empty()
                    {
                        return Err(TranslationConfigError::MissingEzTransPath);
                    }
                    PreparedEngineKind::EzTrans(EzTransProcessConfig {
                        dll_path: resolve_configured_eztrans_path(&config.eztrans_dll_path),
                        dat_path: resolve_configured_eztrans_path(&config.eztrans_dat_path),
                        process_count: crate::config::limits::eztrans_process_count(
                            config.eztrans_process_count,
                        ) as usize,
                    })
                }
                TranslationEngine::Google => PreparedEngineKind::Google,
                TranslationEngine::DeepL => {
                    let keys = config
                        .deepl_effective_keys_with_tiers()
                        .into_iter()
                        .filter(|(key, _)| !key.trim().is_empty())
                        .collect::<Vec<_>>();
                    if keys.is_empty() {
                        return Err(TranslationConfigError::MissingCredential("DeepL API 키"));
                    }
                    PreparedEngineKind::DeepL {
                        keys,
                        strategy: config.deepl_strategy(),
                    }
                }
                TranslationEngine::Papago => {
                    if config.papago_client_id.trim().is_empty()
                        || config.papago_client_secret.trim().is_empty()
                    {
                        return Err(TranslationConfigError::MissingCredential(
                            "Papago client_id/client_secret",
                        ));
                    }
                    PreparedEngineKind::Papago {
                        client_id: config.papago_client_id.clone(),
                        client_secret: config.papago_client_secret.clone(),
                    }
                }
                TranslationEngine::Llm => {
                    if config.llm.api_key.trim().is_empty() {
                        return Err(TranslationConfigError::MissingCredential("LLM API 키"));
                    }
                    PreparedEngineKind::Llm(config.llm.to_call_params().map_err(|error| {
                        TranslationConfigError::InvalidSetting(error.to_string())
                    })?)
                }
                TranslationEngine::Custom => {
                    let custom = config.active_custom_api().map_err(|error| {
                        TranslationConfigError::InvalidSetting(error.to_string())
                    })?;
                    let params = CustomApiCallParams {
                        url: custom.url.clone(),
                        api_key: custom.api_key.clone(),
                        auth_header: custom.auth_header.clone(),
                        auth_scheme: custom.auth_scheme.clone(),
                        headers: custom.headers.clone(),
                        request_template: custom.request_template.clone(),
                        response_path: custom.response_path.clone(),
                    };
                    params
                        .validate()
                        .map_err(TranslationConfigError::InvalidSetting)?;
                    PreparedEngineKind::Custom(params)
                }
            };

        Self::from_kind(kind, source, target)
    }

    fn from_kind(
        kind: PreparedEngineKind,
        source: Language,
        target: Language,
    ) -> Result<Self, TranslationConfigError> {
        let engine = kind.engine();
        if !engine.supports_pair(source, target) {
            return Err(TranslationConfigError::UnsupportedLanguagePair {
                engine: engine.to_str(),
            });
        }
        Ok(Self {
            engine: PreparedEngine(Arc::new(kind)),
            languages: LanguagePair::new(source, target),
        })
    }

    /// EzTrans actor처럼 사전 초기화가 필요한 단문 실행 backend를 준비한다.
    pub fn prepare(&self) -> Result<(), TranslationPrepareError> {
        let Some(config) = self.engine.eztrans_process() else {
            return Ok(());
        };
        prepare_eztrans(&config.dll_path, &config.dat_path)
            .map_err(TranslationPrepareError::EzTransInitialization)
    }

    pub fn engine(&self) -> &PreparedEngine {
        &self.engine
    }

    pub fn languages(&self) -> LanguagePair {
        self.languages
    }
}

/// 설정의 상대 EzTrans 경로는 프로세스의 현재 작업 폴더가 아니라 실행 파일과
/// `config.toml`이 놓인 데이터 폴더를 기준으로 해석한다. 설정값 자체는 변경하지
/// 않으므로 다음 저장에서도 사용자가 입력한 상대 경로가 유지된다.
fn resolve_configured_eztrans_path(configured: &str) -> String {
    let path = std::path::Path::new(configured);
    if path.is_absolute() {
        return configured.to_string();
    }
    crate::runtime::data_dir()
        .join(path)
        .to_string_lossy()
        .into_owned()
}

impl std::fmt::Debug for PreparedJob {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedJob")
            .field("engine", &self.engine)
            .field("languages", &self.languages)
            .finish()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TranslationConfigError {
    #[error("{0}가 설정되지 않았습니다.")]
    MissingCredential(&'static str),
    #[error("EzTrans DLL/DAT 경로가 설정되지 않았습니다.")]
    MissingEzTransPath,
    #[error("{engine} 엔진은 선택한 언어 조합을 지원하지 않습니다.")]
    UnsupportedLanguagePair { engine: &'static str },
    #[error("잘못된 번역 설정: {0}")]
    InvalidSetting(String),
}

#[derive(Debug, thiserror::Error)]
pub enum TranslationPrepareError {
    #[error("EzTrans 초기화 실패: {0}")]
    EzTransInitialization(String),
}

#[cfg(test)]
#[path = "../../tests/unit/translation/job.rs"]
mod tests;
