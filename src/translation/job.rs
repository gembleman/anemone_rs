//! 호출 환경과 무관한 검증된 번역 작업 구성.

use std::sync::{Arc, Mutex, OnceLock};

use crate::config::{EzTransPostprocessEntry, TranslationConfig};
use crate::translation::custom::CustomApiCallParams;
use crate::translation::llm::LlmCallParams;
use crate::translation::{EzTransProcessConfig, Language, TranslationEngine, prepare_eztrans};

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
    EzTrans {
        process: EzTransProcessConfig,
        postprocess_dictionary: Vec<EzTransPostprocessEntry>,
        /// 사전에서 미리 빌드한 automaton — 파일 번역 중 매 결과마다 빌드하지
        /// 않도록 작업 준비 시 한 번만 만든다 (빈 사전이면 `None`).
        postprocess_matcher: Option<super::postprocess::EzTransPostprocessMatcher>,
    },
    Google,
    DeepL {
        keys: Vec<String>,
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
            Self::EzTrans { .. } => TranslationEngine::EzTrans,
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
pub struct PreparedEngine {
    kind: Arc<PreparedEngineKind>,
    /// `cache_engine_id()` 결과 메모이즈. 사전 전체 해시를 요청당 반복하지 않게
    /// 처음 계산한 값만 보관한다 (PreparedJob 전역 재사용과 함께 동작).
    cache_engine_id: OnceLock<String>,
}

impl PreparedEngine {
    fn new(kind: PreparedEngineKind) -> Self {
        Self {
            kind: Arc::new(kind),
            cache_engine_id: OnceLock::new(),
        }
    }

    pub fn engine(&self) -> TranslationEngine {
        self.kind.engine()
    }

    pub fn display_name(&self) -> &'static str {
        self.engine().to_str()
    }

    pub fn max_input_chars(&self) -> usize {
        self.engine().max_input_chars()
    }

    pub fn is_blocking(&self) -> bool {
        matches!(self.kind.as_ref(), PreparedEngineKind::EzTrans { .. })
    }

    pub fn supports_batch(&self) -> bool {
        matches!(self.kind.as_ref(), PreparedEngineKind::EzTrans { .. })
    }

    pub(crate) fn kind(&self) -> &PreparedEngineKind {
        self.kind.as_ref()
    }

    pub(crate) fn eztrans_process(&self) -> Option<&EzTransProcessConfig> {
        match self.kind.as_ref() {
            PreparedEngineKind::EzTrans { process, .. } => Some(process),
            _ => None,
        }
    }

    /// 캐시 키에 쓰이는 엔진 식별자. LLM은 모델이 바뀌면 결과가 달라질 수 있어
    /// 모델명까지 포함하고, 다른 엔진은 엔진명만으로 충분하다.
    pub fn cache_engine_id(&self) -> String {
        self.cache_engine_id
            .get_or_init(|| Self::compute_cache_engine_id(&self.kind))
            .clone()
    }

    fn compute_cache_engine_id(kind: &PreparedEngineKind) -> String {
        match kind {
            PreparedEngineKind::Llm(params) => format!("llm:{}", params.model),
            PreparedEngineKind::EzTrans {
                postprocess_dictionary,
                ..
            } if !postprocess_dictionary.is_empty() => {
                use std::hash::{DefaultHasher, Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                postprocess_dictionary.hash(&mut hasher);
                format!("eztrans:{:016x}", hasher.finish())
            }
            _ => kind.engine().to_str().to_string(),
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
        dictionary_path: String,
        ehnd_path: String,
        process_count: usize,
        source: Language,
        target: Language,
    ) -> Result<Self, TranslationConfigError> {
        if dictionary_path.trim().is_empty() || ehnd_path.trim().is_empty() {
            return Err(TranslationConfigError::MissingEzTransPath);
        }
        Self::from_kind(
            PreparedEngineKind::EzTrans {
                process: EzTransProcessConfig {
                    dictionary_path,
                    ehnd_path,
                    process_count: crate::config::limits::eztrans_process_count_usize(
                        process_count,
                    ),
                },
                postprocess_dictionary: Vec::new(),
                postprocess_matcher: None,
            },
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

    /// `from_config` 결과를 설정 fingerprint로 전역 재사용한다.
    ///
    /// 클립보드 경로는 변경마다 `from_config`를 호출하는데, EzTrans + 후처리
    /// 사전을 쓰면 매번 사전 Vec 전체 clone과 aho-corasick automaton 재빌드가
    /// 일어난다(캐시 hit여도). 같은 설정의 작업은 처음 만든 인스턴스를 공유해
    /// 이 비용을 설정 변경 시 1회로 줄인다. `cache_engine_id` 메모이즈도
    /// 캐시된 인스턴스 안에서 함께 이뤄진다.
    pub fn from_config_cached(
        config: &TranslationConfig,
    ) -> Result<Arc<PreparedJob>, TranslationConfigError> {
        let fingerprint = config_fingerprint(config)?;
        // 캐시는 성능 최적화일 뿐이라 poison되어도 치명적이지 않다 — 다른 스레드가
        // 락을 쥔 채 패닉해도 내용물을 그대로 복구해 계속 쓴다. `expect`로 패닉시키면
        // 이후 모든 클립보드 번역이 이 함수 호출마다 패닉하게 된다.
        let mut cache = PREPARED_JOB_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, job)) = cache.iter().find(|(key, _)| *key == fingerprint) {
            return Ok(job.clone());
        }
        // 락을 잡은 채 from_config를 호출한다 (EzTrans 사전이 크면 aho-corasick
        // automaton 빌드에 수십 ms). 현재 호출자가 UI 스레드 하나뿐이라 당장은
        // 문제가 없지만, 훗날 다른 스레드(예: 파일 번역)에서도 이 함수를 호출하게
        // 되면 그 스레드가 여기서 오래 대기하게 된다는 점을 유의할 것.
        let job = Arc::new(Self::from_config(config)?);
        cache.insert(0, (fingerprint, job.clone()));
        cache.truncate(PREPARED_JOB_CACHE_MAX);
        Ok(job)
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
                    if config.eztrans_dictionary_path.trim().is_empty()
                        || config.eztrans_ehnd_path.trim().is_empty()
                    {
                        return Err(TranslationConfigError::MissingEzTransPath);
                    }
                    PreparedEngineKind::EzTrans {
                        process: EzTransProcessConfig {
                            dictionary_path: resolve_configured_eztrans_path(
                                &config.eztrans_dictionary_path,
                            ),
                            ehnd_path: resolve_configured_eztrans_path(&config.eztrans_ehnd_path),
                            process_count: crate::config::limits::eztrans_process_count(
                                config.eztrans_process_count,
                            ) as usize,
                        },
                        postprocess_dictionary: config.eztrans_postprocess_dictionary.clone(),
                        postprocess_matcher: super::postprocess::EzTransPostprocessMatcher::new(
                            &config.eztrans_postprocess_dictionary,
                        ),
                    }
                }
                TranslationEngine::Google => PreparedEngineKind::Google,
                TranslationEngine::DeepL => {
                    let keys = config
                        .deepl_effective_keys()
                        .into_iter()
                        .filter(|key| !key.trim().is_empty())
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
            engine: PreparedEngine::new(kind),
            languages: LanguagePair::new(source, target),
        })
    }

    /// EzTrans actor처럼 사전 초기화가 필요한 단문 실행 backend를 준비한다.
    pub fn prepare(&self) -> Result<(), TranslationPrepareError> {
        let Some(config) = self.engine.eztrans_process() else {
            return Ok(());
        };
        prepare_eztrans(&config.dictionary_path, &config.ehnd_path)
            .map_err(TranslationPrepareError::EzTransInitialization)
    }

    pub fn engine(&self) -> &PreparedEngine {
        &self.engine
    }

    pub fn languages(&self) -> LanguagePair {
        self.languages
    }

    /// 클립보드 번역 캐시 조회/저장에 쓰는 키를 만든다.
    pub(crate) fn cache_key(&self, original: &str) -> super::CacheKey {
        super::CacheKey {
            engine_id: self.engine.cache_engine_id(),
            source_lang: crate::translation::lang_utils::to_code(self.languages.source()),
            target_lang: crate::translation::lang_utils::to_code(self.languages.target()),
            original: original.to_string(),
        }
    }

    /// 엔진별 후처리를 번역 결과에 적용한다. 현재는 EzTrans 전용 사전만 사용한다.
    pub(crate) fn postprocess(&self, translated: String) -> String {
        match self.engine.kind() {
            PreparedEngineKind::EzTrans {
                postprocess_matcher: Some(matcher),
                ..
            } => matcher.apply(translated),
            _ => translated,
        }
    }
}

/// 설정 fingerprint → 준비된 작업 전역 캐시. 설정 변경은 드물어 소형이면
/// 충분하고, 같은 키 재요청 시 사전 clone/automaton 재빌드를 건너뛴다.
static PREPARED_JOB_CACHE: Mutex<Vec<(u64, Arc<PreparedJob>)>> = Mutex::new(Vec::new());
const PREPARED_JOB_CACHE_MAX: usize = 4;

/// `PreparedJob::from_config`가 소비하는 설정 필드만 반영한 fingerprint.
/// 잘못된 hit(다른 설정이 같은 키)를 막으려면 from_config의 모든 입력을
/// 빠짐없이 해시해야 한다. DefaultHasher면 충분하다 — 입력이 앱 자체 설정이라
/// 공격 대상이 아니다. 사전은 매 요청 한 번 해시되는데, 이는 automaton 재빌드
/// (벤치: 사전 2만 개에서 줄당 27.6ms)보다 2자릿수 이상 저렴한 비용이다.
fn config_fingerprint(config: &TranslationConfig) -> Result<u64, TranslationConfigError> {
    use std::hash::{Hash, Hasher};

    let engine = config
        .get_engine()
        .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
    let source = config
        .get_source_language()
        .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
    let target = config
        .get_target_language()
        .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
    let mut hasher = std::hash::DefaultHasher::new();
    (engine as u8).hash(&mut hasher);
    source.hash(&mut hasher);
    target.hash(&mut hasher);
    match engine {
        TranslationEngine::EzTrans => {
            config.eztrans_dictionary_path.hash(&mut hasher);
            config.eztrans_ehnd_path.hash(&mut hasher);
            config.eztrans_process_count.hash(&mut hasher);
            config.eztrans_postprocess_dictionary.hash(&mut hasher);
        }
        TranslationEngine::Google => {}
        TranslationEngine::DeepL => {
            config.deepl_effective_keys().hash(&mut hasher);
            (config.deepl_strategy() as u8).hash(&mut hasher);
        }
        TranslationEngine::Papago => {
            config.papago_client_id.hash(&mut hasher);
            config.papago_client_secret.hash(&mut hasher);
        }
        TranslationEngine::Llm => {
            let params = config
                .llm
                .to_call_params()
                .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
            (params.provider as u8).hash(&mut hasher);
            params.model.hash(&mut hasher);
            params.api_key.hash(&mut hasher);
            params.base_url.hash(&mut hasher);
            params.system_prompt.hash(&mut hasher);
            params.temperature.to_bits().hash(&mut hasher);
            params.top_p.to_bits().hash(&mut hasher);
            params.frequency_penalty.to_bits().hash(&mut hasher);
            params.presence_penalty.to_bits().hash(&mut hasher);
            params.max_tokens.hash(&mut hasher);
            params
                .reasoning_effort
                .map(|effort| effort as u8)
                .hash(&mut hasher);
            for entry in &params.glossary {
                entry.source.hash(&mut hasher);
                entry.target.hash(&mut hasher);
            }
        }
        TranslationEngine::Custom => {
            let custom = config
                .active_custom_api()
                .map_err(|error| TranslationConfigError::InvalidSetting(error.to_string()))?;
            custom.name.hash(&mut hasher);
            custom.url.hash(&mut hasher);
            custom.api_key.hash(&mut hasher);
            custom.auth_header.hash(&mut hasher);
            custom.auth_scheme.hash(&mut hasher);
            custom.headers.hash(&mut hasher);
            custom.request_template.hash(&mut hasher);
            custom.response_path.hash(&mut hasher);
        }
    }
    Ok(hasher.finish())
}

/// 설정의 상대 EzTrans 경로는 프로세스의 현재 작업 폴더가 아니라 실행 파일과
/// `config.toml`이 놓인 데이터 폴더를 기준으로 해석한다. 설정값 자체는 변경하지
/// 않으므로 다음 저장에서도 사용자가 입력한 상대 경로가 유지된다.
/// 설정에 저장된 EzTrans 경로를 실제 파일 시스템 경로로 해석한다.
/// 상대 경로는 데이터 디렉터리 기준으로 처리한다.
pub fn resolve_configured_eztrans_path(configured: &str) -> String {
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
    #[error("EzTrans 평면 사전/Ehnd 경로가 설정되지 않았습니다.")]
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
