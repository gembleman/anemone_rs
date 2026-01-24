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

    /// 크기 타입으로 크기 가져오기
    pub fn get_size(&self, color_type: ColorType) -> i32 {
        match color_type {
            ColorType::Primary => self.size,
            ColorType::Outline1 => self.outline1_size,
            ColorType::Outline2 => self.outline2_size,
            ColorType::Shadow => 0, // 그림자는 오프셋으로 관리
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

/// 스크린샷 설정
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScreenshotConfig {
    /// 저장 경로
    pub path: String,
    /// 포맷 (0: PNG, 1: JPEG, 2: WebP)
    pub format: u8,
    /// 압축 레벨 (0: 빠름, 1: 표준, 2: 최대)
    pub compression: u8,
    /// JPEG 품질 (1-100)
    pub jpeg_quality: u8,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            format: 0,      // PNG
            compression: 1, // 표준
            jpeg_quality: 85,
        }
    }
}

/// 후크 설정
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookConfig {
    /// 활성화된 후크 목록
    pub active_hooks: Vec<String>,
    /// 비활성화된 후크 목록
    pub inactive_hooks: Vec<String>,
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            active_hooks: vec![
                "클립보드".to_string(),
                "자동저장".to_string(),
                "알림".to_string(),
            ],
            inactive_hooks: vec!["로그".to_string(), "번역기록".to_string()],
        }
    }
}

/// 애플리케이션 설정
#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Config {
    // 윈도우 표시
    pub window_visible: bool,
    pub window_topmost: bool,
    pub temp_window_hide: bool,
    pub click_through: bool,

    // 클립보드
    pub clipboard_watch: bool,
    pub clipboard_max_length: u32,

    // 배경
    pub background_visible: bool,
    pub background_color: u32, // ARGB

    // 테두리
    pub border_visible: bool,
    pub border_width: i32,
    pub border_color: u32, // ARGB

    // 자석 모드
    pub magnetic_mode: bool,
    pub magnetic_minimize: bool,

    // 텍스트 표시
    pub show_name: bool,
    pub show_original: bool,
    pub show_translation: bool,
    pub text_align: TextAlign,

    // 텍스트 스타일 (NAME, ORG, TRANS)
    pub name_style: TextStyle,
    pub original_style: TextStyle,
    pub translation_style: TextStyle,

    // 텍스트 여백
    pub text_margin_x: i32,
    pub text_margin_y: i32,
    pub name_margin: i32,

    // 그림자 오프셋
    pub shadow_offset_x: i32,
    pub shadow_offset_y: i32,

    // 텍스트 반복 처리 모드 (0-4)
    pub repeat_text_mode: u8,

    // 이름 처리 옵션
    pub separate_name: bool,
    pub revise_name: bool,
    pub middle_bracket_recognize: bool,

    // 스크린샷 설정
    pub screenshot: ScreenshotConfig,

    // 후크 설정
    #[serde(default)]
    pub hook: HookConfig,

    // 외부 단축키 사용
    pub extern_hotkey: bool,

    // 숨김 시 클립보드 감시 해제
    pub hide_unwatch_clipboard: bool,
    // 숨김 시 단축키 해제
    pub hide_unlock_hotkey: bool,

    // 업데이트 알림
    pub update_notify: bool,

    // 이전 검색 번호 표시
    pub prev_search_num: bool,

    // AneDic 강제 사용
    pub force_anedic: bool,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum TextAlign {
    Left = 0,
    Center = 1,
    Right = 2,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            window_visible: true,
            window_topmost: true,
            temp_window_hide: false,
            click_through: false,

            clipboard_watch: true,
            clipboard_max_length: 300,

            background_visible: true,
            background_color: 0xC8282828, // 반투명 어두운 배경

            border_visible: true,
            border_width: 2,
            border_color: 0x80AAAAAA,

            magnetic_mode: false,
            magnetic_minimize: true,

            show_name: true,
            show_original: false,
            show_translation: true,
            text_align: TextAlign::Left,

            // 텍스트 스타일
            name_style: TextStyle::default(),
            original_style: TextStyle::default(),
            translation_style: TextStyle::default(),

            // 텍스트 여백
            text_margin_x: 10,
            text_margin_y: 10,
            name_margin: 5,

            // 그림자 오프셋
            shadow_offset_x: 2,
            shadow_offset_y: 2,

            // 텍스트 반복 처리
            repeat_text_mode: 0,

            // 이름 처리
            separate_name: true,
            revise_name: false,
            middle_bracket_recognize: false,

            // 스크린샷
            screenshot: ScreenshotConfig::default(),

            // 후크
            hook: HookConfig::default(),

            // 기타 옵션
            extern_hotkey: false,
            hide_unwatch_clipboard: false,
            hide_unlock_hotkey: false,
            update_notify: true,
            prev_search_num: false,
            force_anedic: false,
        }
    }
}

impl Config {
    pub fn toggle_window_visible(&mut self) {
        self.window_visible = !self.window_visible;
    }

    pub fn toggle_click_through(&mut self) {
        self.click_through = !self.click_through;
    }

    pub fn toggle_clipboard_watch(&mut self) {
        self.clipboard_watch = !self.clipboard_watch;
    }

    pub fn toggle_background_visible(&mut self) {
        self.background_visible = !self.background_visible;
    }

    pub fn toggle_border_visible(&mut self) {
        self.border_visible = !self.border_visible;
    }

    pub fn toggle_magnetic_mode(&mut self) {
        self.magnetic_mode = !self.magnetic_mode;
    }

    /// 텍스트 타입으로 스타일 가져오기 (mutable)
    pub fn get_text_style_mut(&mut self, text_type: TextType) -> &mut TextStyle {
        match text_type {
            TextType::Name => &mut self.name_style,
            TextType::Original => &mut self.original_style,
            TextType::Translation => &mut self.translation_style,
        }
    }

    /// 텍스트 타입으로 스타일 가져오기 (immutable)
    pub fn get_text_style(&self, text_type: TextType) -> &TextStyle {
        match text_type {
            TextType::Name => &self.name_style,
            TextType::Original => &self.original_style,
            TextType::Translation => &self.translation_style,
        }
    }

    /// 텍스트 색상 가져오기
    pub fn get_text_color(&self, text_type: TextType, color_type: ColorType) -> u32 {
        self.get_text_style(text_type).get_color(color_type)
    }

    /// 텍스트 색상 설정
    pub fn set_text_color(&mut self, text_type: TextType, color_type: ColorType, color: u32) {
        self.get_text_style_mut(text_type)
            .set_color(color_type, color);
    }

    /// 텍스트 크기 가져오기
    pub fn get_text_size(&self, text_type: TextType, size_type: ColorType) -> i32 {
        self.get_text_style(text_type).get_size(size_type)
    }

    /// 텍스트 크기 설정
    pub fn set_text_size(&mut self, text_type: TextType, size_type: ColorType, size: i32) {
        self.get_text_style_mut(text_type).set_size(size_type, size);
    }

    /// 모든 텍스트 타입의 크기 동시 설정
    pub fn set_all_text_size(&mut self, size_type: ColorType, size: i32) {
        self.name_style.set_size(size_type, size);
        self.original_style.set_size(size_type, size);
        self.translation_style.set_size(size_type, size);
    }

    /// 그림자 활성화 토글
    pub fn toggle_shadow(&mut self, text_type: TextType) {
        let style = self.get_text_style_mut(text_type);
        style.shadow_enabled = !style.shadow_enabled;
    }

    /// 설정 파일에서 로드
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = serde_json::from_str(&content)?;
        Ok(config)
    }

    /// 설정 파일에 저장
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}
