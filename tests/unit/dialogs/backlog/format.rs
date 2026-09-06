use super::*;

#[test]
fn char_format_contains_segment_and_selected_font_attributes() {
    // richedit.h의 CHARFORMAT2W ABI. EM_SETCHARFORMAT은 이 메모리를
    // 직접 읽으므로 크기와 확장 필드 시작 위치가 정확해야 한다.
    assert_eq!(size_of::<CharFormat2W>(), 116);
    assert_eq!(std::mem::offset_of!(CharFormat2W, face_name), 26);
    assert_eq!(std::mem::offset_of!(CharFormat2W, w_weight), 90);

    let format = make_char_format(Some("Test Font"), 13, true, 0x0012_3456, true);
    assert_eq!(format.cb_size as usize, size_of::<CharFormat2W>());
    assert_eq!(format.cr_text_color, 0x0012_3456);
    assert_eq!(format.y_height, 260);
    assert_eq!(format.dw_effects, CFE_BOLD | CFE_ITALIC);
    assert_eq!(format.w_weight, 700);
    assert_eq!(
        String::from_utf16_lossy(
            &format.face_name[..format.face_name.iter().position(|u| *u == 0).unwrap()]
        ),
        "Test Font"
    );
    assert_ne!(format.dw_mask & CFM_FACE, 0);
}

#[test]
fn char_format_without_selected_face_leaves_face_mask_unset() {
    let format = make_char_format(None, 10, false, 0, false);
    assert_eq!(format.dw_mask & CFM_FACE, 0);
    assert_eq!(format.dw_effects, 0);
}
