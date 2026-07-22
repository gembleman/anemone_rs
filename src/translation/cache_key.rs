/// 캐시 조회/저장 키. LLM은 모델이 바뀌면 결과가 달라질 수 있어 엔진명에 모델을 포함한다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CacheKey {
    pub engine_id: String,
    pub source_lang: &'static str,
    pub target_lang: &'static str,
    pub original: String,
}
