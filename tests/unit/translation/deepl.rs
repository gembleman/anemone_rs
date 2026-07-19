use super::*;

#[test]
fn builds_json_request_with_header_authentication() {
    let request = build_deepl_request(
        &crate::translation::http_common::create_client(),
        "https://api.deepl.com/v2/translate",
        "Hello, World!",
        "EN",
        "KO",
        "secret-key",
    )
    .unwrap();

    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(request.url().as_str(), "https://api.deepl.com/v2/translate");
    assert_eq!(
        request.headers()[reqwest::header::AUTHORIZATION],
        "DeepL-Auth-Key secret-key"
    );
    assert_eq!(
        request.headers()[reqwest::header::CONTENT_TYPE],
        "application/json"
    );

    let body = request.body().and_then(reqwest::Body::as_bytes).unwrap();
    let body: serde_json::Value = serde_json::from_slice(body).unwrap();
    assert_eq!(body["text"], serde_json::json!(["Hello, World!"]));
    assert_eq!(body["source_lang"], "EN");
    assert_eq!(body["target_lang"], "KO");
    assert!(body.get("auth_key").is_none());
}

#[test]
fn selects_endpoint_from_the_api_key_type() {
    assert_eq!(
        deepl_translate_url(DeepLApiTier::Free),
        DEEPL_FREE_TRANSLATE_URL
    );
    assert_eq!(
        deepl_translate_url(DeepLApiTier::Pro),
        DEEPL_PRO_TRANSLATE_URL
    );
}

#[test]
fn rejects_serialized_request_bodies_over_128_kib() {
    let error = build_deepl_request(
        &crate::translation::http_common::create_client(),
        "https://api.deepl.com/v2/translate",
        &"\0".repeat(22_000),
        "EN",
        "KO",
        "secret-key",
    )
    .unwrap_err();

    assert!(matches!(
        error,
        TranslationError::RequestTooLarge {
            engine: "DeepL",
            max_bytes: DEEPL_REQUEST_BODY_LIMIT,
            ..
        }
    ));
}

#[test]
fn parses_the_first_translation_and_ignores_additional_metadata() {
    let response = r#"{
        "translations": [{
            "detected_source_language": "EN",
            "text": "안녕하세요!",
            "billed_characters": 13,
            "model_type_used": "quality_optimized"
        }]
    }"#;

    assert_eq!(parse_deepl_response(response).unwrap(), "안녕하세요!");
}

#[tokio::test]
async fn rejects_a_whitespace_only_api_key() {
    let result = translate_async_with_client(
        &crate::translation::http_common::create_client(),
        "Hello",
        Language::Eng,
        Language::Kor,
        "  \t ",
        DeepLApiTier::Free,
    )
    .await;

    assert!(matches!(result, Err(TranslationError::MissingApiKey)));
}
