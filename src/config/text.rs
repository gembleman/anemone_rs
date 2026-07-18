use serde::{Deserialize, Serialize};

/// 텍스트 유형 (NAME, ORG, TRANS)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum TextType {
    Name = 0,
    Original = 1,
    Translation = 2,
}

/// 색상 유형 (주색상, 외곽선1, 외곽선2, 그림자)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ColorType {
    Primary = 0,  // CFG_A
    Outline1 = 1, // CFG_B
    Outline2 = 2, // CFG_C
    Shadow = 3,   // CFG_S
}

/// 텍스트 스타일 설정
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextStyle {
    /// 폰트 패밀리 이름
    pub font_face: String,
    /// 폰트 스타일 (0: normal, 1: bold, 2: italic, 3: bold+italic)
    pub font_style: u8,
    /// 텍스트 크기
    pub size: i32,
    /// 외곽선1 크기
    pub outline1_size: i32,
    /// 외곽선2 크기
    pub outline2_size: i32,
    /// 그림자 활성화
    pub shadow_enabled: bool,
    /// 주 색상 (ARGB)
    pub color_primary: u32,
    /// 외곽선1 색상 (ARGB)
    pub color_outline1: u32,
    /// 외곽선2 색상 (ARGB)
    pub color_outline2: u32,
    /// 그림자 색상 (ARGB)
    pub color_shadow: u32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_face: "맑은 고딕".to_string(),
            font_style: 0,
            size: 22,
            outline1_size: 2,
            outline2_size: 4,
            shadow_enabled: true,
            color_primary: 0xFFFFFFFF,  // 흰색
            color_outline1: 0xFF000000, // 검정
            color_outline2: 0xFF404040, // 어두운 회색
            color_shadow: 0x80000000,   // 반투명 검정
        }
    }
}

impl TextStyle {
    /// 색상 타입으로 색상 가져오기
    pub fn get_color(&self, color_type: ColorType) -> u32 {
        match color_type {
            ColorType::Primary => self.color_primary,
            ColorType::Outline1 => self.color_outline1,
            ColorType::Outline2 => self.color_outline2,
            ColorType::Shadow => self.color_shadow,
        }
    }

    /// 색상 타입으로 색상 설정
    pub fn set_color(&mut self, color_type: ColorType, color: u32) {
        match color_type {
            ColorType::Primary => self.color_primary = color,
            ColorType::Outline1 => self.color_outline1 = color,
            ColorType::Outline2 => self.color_outline2 = color,
            ColorType::Shadow => self.color_shadow = color,
        }
    }

    /// 크기 타입으로 크기 설정
    pub fn set_size(&mut self, color_type: ColorType, size: i32) {
        match color_type {
            ColorType::Primary => self.size = size,
            ColorType::Outline1 => self.outline1_size = size,
            ColorType::Outline2 => self.outline2_size = size,
            ColorType::Shadow => {} // 그림자는 별도 처리
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TextAlign {
    Left = 0,
    Center = 1,
    Right = 2,
}
