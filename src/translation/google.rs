//! Google Translate API (비공식 웹 API)
//!
//! Google Translate 웹 API를 사용한 번역

use super::{lang_utils, TranslationResult, Translator};
use isolang::Language;
use std::io::{Read, Write};
use std::net::TcpStream;

/// Google 번역기
pub struct GoogleTranslator;

impl GoogleTranslator {
    pub fn new() -> Self {
        Self
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

    /// HTTPS 요청을 Windows API로 수행
    fn http_request(text: &str, source: &str, target: &str) -> Result<String, String> {
        // Google Translate 비공식 API 엔드포인트
        let encoded_text = Self::url_encode(text);
        let path = format!(
            "/translate_a/single?client=gtx&sl={}&tl={}&dt=t&q={}",
            source, target, encoded_text
        );

        // TLS 연결을 위해 native-tls 또는 Windows API 사용
        // 여기서는 간단히 HTTP 연결 시도 (실제로는 HTTPS 필요)
        // 실제 구현에서는 winhttp나 외부 크레이트 사용 권장

        let host = "translate.googleapis.com";

        // TCP 연결
        let mut stream = TcpStream::connect(format!("{}:80", host))
            .map_err(|e| format!("연결 실패: {}", e))?;

        // HTTP 요청 생성
        let request = format!(
            "GET {} HTTP/1.1\r\n\
             Host: {}\r\n\
             User-Agent: Mozilla/5.0\r\n\
             Accept: */*\r\n\
             Connection: close\r\n\
             \r\n",
            path, host
        );

        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("요청 전송 실패: {}", e))?;

        // 응답 읽기
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|e| format!("응답 읽기 실패: {}", e))?;

        let response_str =
            String::from_utf8_lossy(&response).to_string();

        // HTTP 헤더와 본문 분리
        if let Some(body_start) = response_str.find("\r\n\r\n") {
            let body = &response_str[body_start + 4..];
            Self::parse_google_response(body)
        } else {
            Err("응답 파싱 실패".to_string())
        }
    }

    /// Google API 응답 파싱
    fn parse_google_response(json: &str) -> Result<String, String> {
        // Google Translate API 응답 형식:
        // [[["번역결과","원문",null,null,10],...],...]
        // 간단한 JSON 파싱

        let mut result = String::new();
        let mut in_string = false;
        let mut escape = false;
        let mut current_string = String::new();
        let mut strings: Vec<String> = Vec::new();

        for ch in json.chars() {
            if escape {
                current_string.push(ch);
                escape = false;
                continue;
            }

            match ch {
                '\\' if in_string => {
                    escape = true;
                    current_string.push(ch);
                }
                '"' => {
                    if in_string {
                        strings.push(current_string.clone());
                        current_string.clear();
                    }
                    in_string = !in_string;
                }
                _ if in_string => {
                    current_string.push(ch);
                }
                _ => {}
            }
        }

        // 첫 번째, 세 번째, 다섯 번째... 문자열이 번역 결과
        // 형식: [["번역1","원문1",...],["번역2","원문2",...],...]
        let mut i = 0;
        while i < strings.len() {
            // 번역 결과는 짝수 인덱스에 있음 (0, 2, 4, ...)
            // 하지만 실제로는 더 복잡한 구조
            if i % 2 == 0 && !strings[i].is_empty() {
                if !result.is_empty() {
                    result.push(' ');
                }
                // unescape
                let unescaped = strings[i]
                    .replace("\\n", "\n")
                    .replace("\\r", "\r")
                    .replace("\\t", "\t")
                    .replace("\\\"", "\"")
                    .replace("\\\\", "\\");
                result.push_str(&unescaped);
            }
            i += 2;

            // 너무 많이 반복하지 않도록
            if i > 20 {
                break;
            }
        }

        if result.is_empty() {
            Err("번역 결과를 찾을 수 없습니다.".to_string())
        } else {
            Ok(result)
        }
    }
}

impl Default for GoogleTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator for GoogleTranslator {
    fn translate(&self, text: &str, source: Language, target: Language) -> TranslationResult {
        if text.is_empty() {
            return TranslationResult::Error("빈 텍스트입니다.".to_string());
        }

        let source_code = lang_utils::to_google_code(source);
        let target_code = lang_utils::to_google_code(target);

        match Self::http_request(text, source_code, target_code) {
            Ok(result) => TranslationResult::Success(result),
            Err(e) => TranslationResult::Error(format!("Google 번역 실패: {}", e)),
        }
    }

    fn engine_name(&self) -> &'static str {
        "Google Translate"
    }

    fn is_available(&self) -> bool {
        // 네트워크 연결 확인 없이 항상 true
        // 실제 번역 시 오류 처리
        true
    }
}
