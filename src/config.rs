/// 애플리케이션 설정
#[derive(Clone)]
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
}

#[derive(Clone, Copy, PartialEq)]
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
}
