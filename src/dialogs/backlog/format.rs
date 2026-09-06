//! RichEdit `CHARFORMAT2W` ABI 정의와 서식 값 생성.

const CFM_BOLD: u32 = 0x0000_0001;
const CFM_ITALIC: u32 = 0x0000_0002;
const CFM_SIZE: u32 = 0x8000_0000;
const CFM_COLOR: u32 = 0x4000_0000;
const CFM_FACE: u32 = 0x2000_0000;
const CFE_BOLD: u32 = 0x0000_0001;
const CFE_ITALIC: u32 = 0x0000_0002;
const LF_FACESIZE: usize = 32;

/// RichEdit 2.0+의 EM_SETCHARFORMAT에 전달하는 CHARFORMAT2W.
///
/// windows-sys는 RichEdit 전용 구조체를 노출하지 않으므로 C 헤더와 같은
/// repr(C) 레이아웃을 현장에서 정의한다.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct CharFormat2W {
    pub(super) cb_size: u32,
    pub(super) dw_mask: u32,
    pub(super) dw_effects: u32,
    pub(super) y_height: i32,
    pub(super) y_offset: i32,
    pub(super) cr_text_color: u32,
    pub(super) b_char_set: u8,
    pub(super) b_pitch_and_family: u8,
    pub(super) face_name: [u16; LF_FACESIZE],
    pub(super) w_weight: u16,
    pub(super) s_spacing: i16,
    pub(super) cr_back_color: u32,
    pub(super) lcid: u32,
    pub(super) dw_reserved: u32,
    pub(super) s_style: i16,
    pub(super) w_kerning: u16,
    pub(super) b_underline_type: u8,
    pub(super) b_animation: u8,
    pub(super) b_rev_author: u8,
    pub(super) b_reserved1: u8,
}

pub(super) fn make_char_format(
    face_name: Option<&str>,
    point_size: i32,
    italic: bool,
    color: u32,
    bold: bool,
) -> CharFormat2W {
    let mut face = [0u16; LF_FACESIZE];
    let mut mask = CFM_COLOR | CFM_BOLD | CFM_ITALIC | CFM_SIZE;
    if let Some(name) = face_name {
        let units: Vec<u16> = name.encode_utf16().take(LF_FACESIZE - 1).collect();
        face[..units.len()].copy_from_slice(&units);
        mask |= CFM_FACE;
    }
    let mut effects = 0;
    if bold {
        effects |= CFE_BOLD;
    }
    if italic {
        effects |= CFE_ITALIC;
    }
    CharFormat2W {
        cb_size: size_of::<CharFormat2W>() as u32,
        dw_mask: mask,
        dw_effects: effects,
        y_height: point_size.max(1) * 20,
        y_offset: 0,
        cr_text_color: color,
        b_char_set: 0,
        b_pitch_and_family: 0,
        face_name: face,
        w_weight: if bold { 700 } else { 400 },
        s_spacing: 0,
        cr_back_color: 0,
        lcid: 0,
        dw_reserved: 0,
        s_style: 0,
        w_kerning: 0,
        b_underline_type: 0,
        b_animation: 0,
        b_rev_author: 0,
        b_reserved1: 0,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/dialogs/backlog/format.rs"]
mod tests;
