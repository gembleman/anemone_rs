use super::*;

#[test]
fn composition_retry_uses_bounded_exponential_backoff() {
    assert_eq!(composition_retry_delay_ms(0), 250);
    assert_eq!(composition_retry_delay_ms(1), 500);
    assert_eq!(composition_retry_delay_ms(5), 8_000);
    assert_eq!(composition_retry_delay_ms(100), 8_000);
}

#[test]
fn display_segments_apply_name_and_visibility_policies() {
    let config = crate::config::Config {
        show_name: true,
        show_original: true,
        show_translation: true,
        separate_name: true,
        ..Default::default()
    };

    let segments = display_segments(&config, "Alice: Hello", "앨리스: 안녕");

    assert_eq!(segments.len(), 3);
    assert_eq!(
        segments[0],
        (crate::config::TextType::Name, "앨리스".into())
    );
    assert_eq!(
        segments[1],
        (crate::config::TextType::Original, "Hello".into())
    );
    assert_eq!(
        segments[2],
        (crate::config::TextType::Translation, "안녕".into())
    );
}

#[test]
fn render_blocks_use_each_text_style_and_name_margin() {
    let mut config = crate::config::Config {
        show_original: true,
        show_translation: true,
        name_margin: 17,
        ..Default::default()
    };
    config.name_style.size = 11;
    config.original_style.size = 22;
    config.translation_style.size = 33;

    let blocks = build_render_blocks(&config, "Name: source", "이름: target");

    assert_eq!(
        blocks
            .iter()
            .map(|block| block.style.font_size)
            .collect::<Vec<_>>(),
        [11, 22, 33]
    );
    assert!(blocks[1].top >= blocks[0].top + blocks[0].height + 17.0);
}
