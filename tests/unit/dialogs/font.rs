use super::*;

#[test]
fn test_font_style_bits() {
    let style = FontStyle::from_bits(3);
    assert!(style.bold);
    assert!(style.italic);
    assert_eq!(style.to_bits(), 3);

    let style2 = FontStyle::from_bits(1);
    assert!(style2.bold);
    assert!(!style2.italic);
}
