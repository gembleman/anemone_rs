use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{ColorType, HotkeyConfig, TextAlign, TextStyle, TextType, TranslationConfig};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// 애플리케이션 설정
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 설정 구조 버전. 누락된 기존 설정은 v0 migration을 거친다.
    #[serde(default)]
    pub schema_version: u32,

    // 윈도우 표시
    pub window_visible: bool,
    pub window_topmost: bool,
    pub click_through: bool,

    // 클립보드
    pub clipboard_watch: bool,
    pub clipboard_max_length: u32,
    /// 동일 원문 재번역 시 API 호출을 건너뛰고 sqlite 캐시를 사용할지 여부.
    #[serde(default = "default_clipboard_cache_enabled")]
    pub clipboard_cache_enabled: bool,

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

    // 이름 처리 옵션
    pub separate_name: bool,

    // 번역 설정
    #[serde(default)]
    pub translation: TranslationConfig,

    /// 전역 단축키 설정. 기존 config.toml(필드 없음)은 기본값(Ctrl+Shift+A/Up/Down/C)으로 채워진다.
    #[serde(default)]
    pub hotkeys: HotkeyConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            window_visible: true,
            window_topmost: true,
            click_through: false,

            clipboard_watch: true,
            clipboard_max_length: 300,
            clipboard_cache_enabled: true,

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

            // 이름 처리
            separate_name: true,

            // 번역
            translation: TranslationConfig::default(),

            // 단축키
            hotkeys: HotkeyConfig::default(),
        }
    }
}

fn default_clipboard_cache_enabled() -> bool {
    true
}

impl Config {
    /// 외부 설정값을 UI 범위로 정규화해 모든 소비자가 같은 값을 보게 한다.
    pub fn normalize(&mut self) {
        self.border_width = super::limits::border_width(self.border_width);
        self.text_margin_x = super::limits::margin(self.text_margin_x);
        self.text_margin_y = super::limits::margin(self.text_margin_y);
        self.name_margin = super::limits::margin(self.name_margin);
        self.shadow_offset_x = super::limits::shadow_offset(self.shadow_offset_x);
        self.shadow_offset_y = super::limits::shadow_offset(self.shadow_offset_y);
        self.translation.eztrans_process_count =
            super::limits::eztrans_process_count(self.translation.eztrans_process_count);

        for style in [
            &mut self.name_style,
            &mut self.original_style,
            &mut self.translation_style,
        ] {
            style.size = super::limits::text_size(style.size);
            style.outline1_size = super::limits::outline_size(style.outline1_size);
            style.outline2_size = super::limits::outline_size(style.outline2_size);
            style.font_style &= 0b11;
        }

        let llm = &mut self.translation.llm;
        llm.max_tokens = super::limits::llm_max_tokens(llm.max_tokens);
        llm.debounce_ms = super::limits::llm_debounce_ms(llm.debounce_ms);
        llm.temperature = super::limits::llm_temperature(llm.temperature);
        llm.top_p = super::limits::llm_top_p(llm.top_p);
        llm.frequency_penalty = super::limits::llm_penalty(llm.frequency_penalty);
        llm.presence_penalty = super::limits::llm_penalty(llm.presence_penalty);
    }

    fn migrate(&mut self) -> Result<(), ConfigDecodeError> {
        if self.schema_version > CURRENT_SCHEMA_VERSION {
            return Err(ConfigDecodeError::UnsupportedSchema {
                found: self.schema_version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }

        if self.schema_version == 0 {
            self.translation.migrate_legacy_custom_api();
            self.schema_version = 1;
        }

        Ok(())
    }

    pub fn toggle_window_visible(&mut self) {
        self.window_visible = !self.window_visible;
    }

    pub fn toggle_click_through(&mut self) {
        self.click_through = !self.click_through;
    }

    pub fn toggle_background_visible(&mut self) {
        self.background_visible = !self.background_visible;
    }

    pub fn toggle_border_visible(&mut self) {
        self.border_visible = !self.border_visible;
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

    /// 모든 텍스트 타입의 크기 동시 설정
    pub fn set_all_text_size(&mut self, size_type: ColorType, size: i32) {
        self.name_style.set_size(size_type, size);
        self.original_style.set_size(size_type, size);
        self.translation_style.set_size(size_type, size);
    }

    /// 설정 파일에서 로드 (TOML 형식)
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, ConfigLoadError> {
        let content = std::fs::read_to_string(path)?;
        Ok(Self::from_toml_str(&content)?)
    }

    fn from_toml_str(content: &str) -> Result<Self, ConfigDecodeError> {
        let mut config: Config = toml::from_str(content)?;
        config.migrate()?;
        config.normalize();
        Ok(config)
    }

    /// 설정 파일에 저장 (TOML 형식)
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        self.save_to_file_impl(path, true)
    }

    fn save_to_file_impl(
        &self,
        path: &std::path::Path,
        preserve_previous: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut normalized = self.clone();
        normalized.migrate()?;
        normalized.normalize();
        let content = toml::to_string_pretty(&normalized)?;
        let _: Config = Self::from_toml_str(&content)?;

        // 교체 전에 정상본을 보존하며 손상된 파일로 backup을 덮지 않는다.
        if preserve_previous && path.is_file() && Self::load_from_file(path).is_ok() {
            let backup_path = path.with_extension("toml.last-good");
            let previous = std::fs::read(path)?;
            crate::fs_util::atomic_write(&backup_path, &previous)?;
        }
        crate::fs_util::atomic_write(path, content.as_bytes())?;
        Ok(())
    }

    /// 기본 설정 파일 경로 가져오기 (캐시됨)
    pub fn default_config_path() -> &'static PathBuf {
        use std::sync::OnceLock;
        static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();
        CONFIG_PATH.get_or_init(|| crate::runtime::paths().config_file())
    }

    /// 기본 경로에서 설정 로드 (없거나 파싱 에러 시 기본값 사용)
    pub(crate) fn load_or_default() -> Self {
        let path = Self::default_config_path();
        Self::load_or_default_from(path)
    }

    fn load_or_default_from(path: &std::path::Path) -> Self {
        if !path.exists() {
            tracing::info!("설정 파일 없음, 기본 설정 생성: {}", path.display());
            let config = Self::default();
            if let Err(e) = config.save_to_file(path) {
                tracing::error!("기본 설정 파일 생성 실패: {}", e);
            }
            return config;
        }

        match Self::load_from_file(path) {
            Ok(mut config) => {
                if relocate_missing_bundled_eztrans_paths(&mut config, path) {
                    tracing::info!(
                        "실행 파일 위치에 맞춰 EzTrans 번들 경로를 갱신했습니다: {}",
                        path.display()
                    );
                    if let Err(error) = config.save_to_file(path) {
                        tracing::warn!("갱신한 EzTrans 번들 경로 저장 실패: {error}");
                    }
                }
                tracing::info!("설정 로드됨: {}", path.display());
                config
            }
            Err(ConfigLoadError::Decode(_)) => {
                // Parser 오류에 API key가 섞일 수 있으므로 세부 본문은 기록하지 않는다.
                tracing::error!("설정 파일을 파싱할 수 없습니다");
                tracing::warn!("기본 설정으로 시작합니다.");

                // 손상본을 격리해 원문이나 `.last-good`에서 복구할 수 있게 한다.
                match quarantine_corrupt_file(path) {
                    Ok(quarantine) => tracing::warn!(
                        "손상된 설정을 격리했습니다. 복구 파일: {}",
                        quarantine.display()
                    ),
                    Err(error) => {
                        tracing::error!("손상된 설정 격리 실패(원본은 덮어쓰지 않음): {error}")
                    }
                }
                Self::default()
            }
            Err(ConfigLoadError::Io(error)) => {
                tracing::error!("설정 파일을 읽을 수 없습니다: {error}");
                tracing::warn!("원본 설정을 보존하고 기본 설정으로 시작합니다.");
                Self::default()
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

/// 다른 설치 폴더나 worktree에서 가져온 설정이 더 이상 존재하지 않는 번들 경로를
/// 가리키면 현재 실행 파일 옆에 배치된 번들로 연결한다. 명시적인 사용자 경로는
/// `eztrans_dll/J2KEngine.dll` + `eztrans_dll/Dat` 형태일 때만 보정한다.
fn relocate_missing_bundled_eztrans_paths(
    config: &mut Config,
    config_path: &std::path::Path,
) -> bool {
    let dll_path = std::path::Path::new(&config.translation.eztrans_dll_path);
    let dat_path = std::path::Path::new(&config.translation.eztrans_dat_path);
    if !dll_path.is_absolute() || !dat_path.is_absolute() {
        return false;
    }
    if dll_path.is_file() && dat_path.is_dir() {
        return false;
    }
    if !is_bundled_eztrans_path(dll_path, "J2KEngine.dll")
        || !is_bundled_eztrans_path(dat_path, "Dat")
    {
        return false;
    }

    let Some(config_dir) = config_path.parent() else {
        return false;
    };
    let bundled_dir = config_dir.join("eztrans_dll");
    let bundled_dll = bundled_dir.join("J2KEngine.dll");
    let bundled_dat = bundled_dir.join("Dat");
    if !bundled_dll.is_file() || !bundled_dat.is_dir() {
        return false;
    }

    config.translation.eztrans_dll_path = bundled_dll.to_string_lossy().into_owned();
    config.translation.eztrans_dat_path = bundled_dat.to_string_lossy().into_owned();
    true
}

fn is_bundled_eztrans_path(path: &std::path::Path, leaf: &str) -> bool {
    path.file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case(leaf))
        && path
            .parent()
            .and_then(std::path::Path::file_name)
            .is_some_and(|name| name.eq_ignore_ascii_case("eztrans_dll"))
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigLoadError {
    #[error("설정 파일 읽기 실패: {0}")]
    Io(#[from] std::io::Error),
    #[error("설정 TOML 파싱 실패: {0}")]
    Decode(#[from] ConfigDecodeError),
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigDecodeError {
    #[error("설정 TOML 파싱 실패: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("지원하지 않는 설정 schema_version {found} (최대 지원: {supported})")]
    UnsupportedSchema { found: u32, supported: u32 },
}

fn quarantine_corrupt_file(path: &std::path::Path) -> std::io::Result<PathBuf> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    for suffix in 0..1000u16 {
        let postfix = if suffix == 0 {
            timestamp.to_string()
        } else {
            format!("{timestamp}-{suffix}")
        };
        let quarantine = path.with_file_name(format!("{stem}.corrupt-{postfix}"));
        if !quarantine.exists() {
            std::fs::rename(path, &quarantine)?;
            return Ok(quarantine);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "설정 격리 파일 이름을 할당할 수 없습니다",
    ))
}

#[cfg(test)]
#[path = "../../tests/unit/config/app.rs"]
mod tests;
