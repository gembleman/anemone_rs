use super::{format_deepl_key, mask_secret};

#[test]
fn secret_mask_never_contains_the_complete_secret() {
    let masked = mask_secret("super-secret-1234");
    assert_eq!(masked, "••••1234");
    assert!(!masked.contains("super-secret"));
    assert_eq!(mask_secret("abc"), "••••");
}

#[test]
fn deepl_key_labels_show_the_detected_api_tier() {
    assert_eq!(format_deepl_key("free-secret:fx"), "[무료] ••••t:fx");
    assert_eq!(format_deepl_key("pro-secret"), "[유료] ••••cret");
}
