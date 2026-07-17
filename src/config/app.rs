use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{ColorType, HookConfig, TextAlign, TextStyle, TextType, TranslationConfig};

/// 애플리케이션 설정
#[derive(Clone, Debug, Serialize, Deserialize)]
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

    // 후크 설정
    #[serde(default)]
    pub hook: HookConfig,

    // 번역 설정
    #[serde(default)]
    pub translation: TranslationConfig,

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

            // 후크
            hook: HookConfig::default(),

            // 번역
            translation: TranslationConfig::default(),

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

    /// 설정 파일에서 로드 (TOML 형식)
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// 설정 파일에 저장 (TOML 형식)
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// 기본 설정 파일 경로 가져오기 (캐시됨)
    pub fn default_config_path() -> &'static PathBuf {
        use std::sync::OnceLock;
        static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();
        CONFIG_PATH.get_or_init(|| {
            if let Ok(exe_path) = std::env::current_exe()
                && let Some(exe_dir) = exe_path.parent()
            {
                return exe_dir.join("config.toml");
            }
            PathBuf::from("config.toml")
        })
    }

    /// 기본 경로에서 설정 로드 (없거나 파싱 에러 시 기본값 사용)
    pub fn load_or_default() -> Self {
        let path = Self::default_config_path();

        // 파일이 존재하는지 확인
        if !path.exists() {
            tracing::info!("설정 파일 없음, 기본 설정 생성: {}", path.display());
            let config = Self::default();
            if let Err(e) = config.save() {
                tracing::error!("기본 설정 파일 생성 실패: {}", e);
            }
            return config;
        }

        // 파일 로드 시도
        match Self::load_from_file(path) {
            Ok(config) => {
                tracing::info!("설정 로드됨: {}", path.display());
                config
            }
            Err(e) => {
                tracing::error!("설정 파일 파싱 에러: {}", e);
                tracing::warn!("기본 설정으로 시작합니다.");

                // 손상된 설정 파일 백업
                let backup_path = path.with_extension("toml.bak");
                if let Err(backup_err) = std::fs::copy(path, &backup_path) {
                    tracing::error!("설정 파일 백업 실패: {}", backup_err);
                } else {
                    tracing::info!("기존 설정 파일 백업됨: {}", backup_path.display());
                }

                // 기본 설정으로 덮어쓰기
                let config = Self::default();
                if let Err(save_err) = config.save() {
                    tracing::error!("기본 설정 파일 생성 실패: {}", save_err);
                }
                config
            }
        }
    }

    /// 기본 경로에 설정 저장
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::default_config_path();
        self.save_to_file(path)?;
        tracing::debug!("설정 저장됨: {}", path.display());
        Ok(())
    }
}
