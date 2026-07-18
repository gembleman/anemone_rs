use serde::{Deserialize, Serialize};

/// 임의의 JSON 기반 REST 번역 API 설정.
///
/// 요청 템플릿의 문자열 값에서는 `{text}`, `{source}`, `{target}`, `{api_key}`를
/// 사용할 수 있다. 응답 경로가 비어 있으면 응답 본문 전체를 번역문으로 사용한다.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomApiConfig {
    /// POST 요청을 보낼 전체 URL.
    #[serde(default)]
    pub url: String,
    /// 선택적 API 키. 헤더 또는 요청 템플릿의 `{api_key}`로 전달할 수 있다.
    #[serde(default)]
    pub api_key: String,
    /// API 키를 보낼 HTTP 헤더 이름. 비우면 인증 헤더를 보내지 않는다.
    #[serde(default = "default_auth_header")]
    pub auth_header: String,
    /// API 키 앞에 붙일 인증 스킴. 비우면 키만 헤더 값으로 보낸다.
    #[serde(default = "default_auth_scheme")]
    pub auth_scheme: String,
    /// 추가 HTTP 헤더 JSON 객체. 문자열 값에서 요청 템플릿과 같은 변수를 쓸 수 있다.
    #[serde(default = "default_headers")]
    pub headers: String,
    /// JSON 요청 본문 템플릿.
    #[serde(default = "default_request_template")]
    pub request_template: String,
    /// 번역문이 들어 있는 JSON 경로. `/data/text` 또는 `data.text` 형식.
    #[serde(default = "default_response_path")]
    pub response_path: String,
}

fn default_auth_header() -> String {
    "Authorization".to_string()
}

fn default_auth_scheme() -> String {
    "Bearer".to_string()
}

fn default_request_template() -> String {
    r#"{"text":"{text}","source":"{source}","target":"{target}"}"#.to_string()
}

fn default_headers() -> String {
    "{}".to_string()
}

fn default_response_path() -> String {
    "translatedText".to_string()
}

impl Default for CustomApiConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            api_key: String::new(),
            auth_header: default_auth_header(),
            auth_scheme: default_auth_scheme(),
            headers: default_headers(),
            request_template: default_request_template(),
            response_path: default_response_path(),
        }
    }
}
