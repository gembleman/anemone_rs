//! API 키·토큰처럼 화면에 그대로 보여줄 수 없는 값의 마스킹 표시.

/// 비밀 값을 마스킹해 마지막 4글자만 보여준다. 4글자 이하면 아예 숨긴다.
pub(super) fn mask_secret(secret: &str) -> String {
    if secret.is_empty() {
        return String::new();
    }
    if secret.chars().count() <= 4 {
        return "••••".to_string();
    }
    let suffix: String = secret
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("••••{suffix}")
}

/// DeepL API key 목록 항목 표시용: 무료/유료 등급과 마스킹된 키를 함께 보여준다.
pub(super) fn format_deepl_key(secret: &str) -> String {
    let tier = crate::translation::DeepLApiTier::from_api_key(secret);
    format!("[{}] {}", tier.display_name(), mask_secret(secret))
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/settings/secret_format.rs"]
mod tests;
