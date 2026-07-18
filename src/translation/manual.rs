//! 수동 번역 다이얼로그의 Win32 비의존 입력·출력 변환 규칙.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ManualTranslationOptions {
    pub remove_linefeeds: bool,
    pub output_format: ManualOutputFormat,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ManualOutputFormat {
    #[default]
    Normal,
    Brackets,
    NameSplit,
}

impl ManualTranslationOptions {
    pub fn prepare_input(&self, source: &str) -> String {
        if self.remove_linefeeds {
            source.replace("\r\n", " ").replace('\n', " ")
        } else {
            source.to_string()
        }
    }

    pub fn format_output(&self, translated: String) -> String {
        match self.output_format {
            ManualOutputFormat::Normal => translated,
            ManualOutputFormat::Brackets => format!("「{translated}」"),
            ManualOutputFormat::NameSplit => {
                if let Some((name, body)) = translated.split_once([':', '：']) {
                    format!("{}\n{}", name.trim(), body.trim())
                } else {
                    translated
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/translation/manual.rs"]
mod tests;
