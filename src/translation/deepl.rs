//! DeepL API 번역 엔진
//!
//! DeepL API를 사용한 번역 (API 키 필요)

use super::{lang_utils, TranslationResult, Translator};
use isolang::Language;
use std::io::{Read, Write};
use std::net::TcpStream;

/// DeepL 번역기
pub struct DeepLTranslator {
    api_key: String,
}

impl DeepLTranslator {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    /// URL 인코딩
    fn url_encode(s: &str) -> String {
        let mut result = String::new();
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(byte as char);
                }
                b' ' => {
                    result.push_str("%20");
                }
                _ => {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        result
    }

    /// DeepL API 호출
    fn call_api(&self, text: &str, source: &str, target: &str) -> Result<String, String> {
        if self.api_key.is_empty() {
            return Err("DeepL API 키가 설정되지 않았습니다.".to_string());
        }

        // DeepL API 엔드포인트 (Free API는 api-free.deepl.com)
        let host = if self.api_key.ends_with(":fx") {
            "api-free.deepl.com"
        } else {
            "api.deepl.com"
        };

        // POST 데이터
        let encoded_text = Self::url_encode(text);
        let body = format!(
            "auth_key={}&text={}&source_lang={}&target_lang={}",
            Self::url_encode(&self.api_key),
            encoded_text,
            source,
            target
        );

        // TCP 연결 (실제로는 HTTPS 필요)
        let mut stream = TcpStream::connect(format!("{}:80", host))
            .map_err(|e| format!("연결 실패: {}", e))?;

        // HTTP 요청
        let request = format!(
            "POST /v2/translate HTTP/1.1\r\n\
             Host: {}\r\n\
             Content-Type: application/x-www-form-urlencoded\r\n\
             Content-Length: {}\r\n\
             User-Agent: AnemoneRS/1.0\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            host,
            body.len(),
            body
        );

        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("요청 전송 실패: {}", e))?;

        // 응답 읽기
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|e| format!("응답 읽기 실패: {}", e))?;

        let response_str = String::from_utf8_lossy(&response).to_string();

        // HTTP 헤더와 본문 분리
        if let Some(body_start) = response_str.find("\r\n\r\n") {
            let body = &response_str[body_start + 4..];
            Self::parse_deepl_response(body)
        } else {
            Err("응답 파싱 실패".to_string())
        }
    }

    /// DeepL API 응답 파싱
    fn parse_deepl_response(json: &str) -> Result<String, String> {
        // DeepL API 응답 형식:
        // {"translations":[{"detected_source_language":"JA","text":"번역결과"}]}

        // "text":" 찾기
        if let Some(text_start) = json.find("\"text\":\"") {
            let start = text_start + 8;
            let remaining = &json[start..];

            // 닫는 따옴표 찾기 (이스케이프 처리)
            let mut result = String::new();
            let mut escape = false;

            for ch in remaining.chars() {
                if escape {
                    match ch {
                        'n' => result.push('\n'),
                        'r' => result.push('\r'),
                        't' => result.push('\t'),
                        '"' => result.push('"'),
                        '\\' => result.push('\\'),
                        _ => {
                            result.push('\\');
                            result.push(ch);
                        }
                    }
                    escape = false;
                } else if ch == '\\' {
                    escape = true;
                } else if ch == '"' {
                    break;
                } else {
                    result.push(ch);
                }
            }

            if result.is_empty() {
                Err("빈 번역 결과".to_string())
            } else {
                Ok(result)
            }
        } else if json.contains("\"message\"") {
            // 에러 응답
            if let Some(msg_start) = json.find("\"message\":\"") {
                let start = msg_start + 11;
                let remaining = &json[start..];
                if let Some(end) = remaining.find('"') {
                    return Err(format!("DeepL 에러: {}", &remaining[..end]));
                }
            }
            Err("DeepL API 에러".to_string())
        } else {
            Err("번역 결과를 찾을 수 없습니다.".to_string())
        }
    }
}

impl Translator for DeepLTranslator {
    fn translate(&self, text: &str, source: Language, target: Language) -> TranslationResult {
        if text.is_empty() {
            return TranslationResult::Error("빈 텍스트입니다.".to_string());
        }

        if self.api_key.is_empty() {
            return TranslationResult::Error("DeepL API 키가 설정되지 않았습니다.".to_string());
        }

        let source_code = lang_utils::to_deepl_code(source);
        let target_code = lang_utils::to_deepl_code(target);

        match self.call_api(text, source_code, target_code) {
            Ok(result) => TranslationResult::Success(result),
            Err(e) => TranslationResult::Error(e),
        }
    }

    fn engine_name(&self) -> &'static str {
        "DeepL"
    }

    fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }
}
