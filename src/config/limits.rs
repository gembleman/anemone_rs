//! 설정 파일, 편집기와 실행 준비가 공유하는 값 범위.

pub const BORDER_WIDTH_MIN: i32 = 0;
pub const BORDER_WIDTH_MAX: i32 = 10;
pub const MARGIN_MIN: i32 = 0;
pub const MARGIN_MAX: i32 = 300;
pub const SHADOW_OFFSET_MIN: i32 = 0;
pub const SHADOW_OFFSET_MAX: i32 = 20;
pub const TEXT_SIZE_MIN: i32 = 6;
pub const TEXT_SIZE_MAX: i32 = 100;
pub const OUTLINE_SIZE_MIN: i32 = 0;
pub const OUTLINE_SIZE_MAX: i32 = 20;

pub const EZTRANS_PROCESS_COUNT_MIN: u32 = 1;
pub const EZTRANS_PROCESS_COUNT_MAX: u32 = 16;
pub const LLM_MAX_TOKENS_MIN: u32 = 1;
pub const LLM_MAX_TOKENS_MAX: u32 = 32_000;
pub const LLM_DEBOUNCE_MS_MIN: u32 = 0;
pub const LLM_DEBOUNCE_MS_MAX: u32 = 10_000;
pub const LLM_TEMPERATURE_MIN: f32 = 0.0;
pub const LLM_TEMPERATURE_MAX: f32 = 2.0;
pub const LLM_TEMPERATURE_DEFAULT: f32 = 1.0;
pub const LLM_TOP_P_MIN: f32 = 0.0;
pub const LLM_TOP_P_MAX: f32 = 1.0;
pub const LLM_TOP_P_DEFAULT: f32 = 1.0;
pub const LLM_PENALTY_MIN: f32 = -2.0;
pub const LLM_PENALTY_MAX: f32 = 2.0;
pub const LLM_PENALTY_DEFAULT: f32 = 0.0;
pub const LLM_TEMPERATURE_SLIDER_MIN: i32 = 0;
pub const LLM_TEMPERATURE_SLIDER_MAX: i32 = 200;

pub fn border_width(value: i32) -> i32 {
    value.clamp(BORDER_WIDTH_MIN, BORDER_WIDTH_MAX)
}

pub fn margin(value: i32) -> i32 {
    value.clamp(MARGIN_MIN, MARGIN_MAX)
}

pub fn shadow_offset(value: i32) -> i32 {
    value.clamp(SHADOW_OFFSET_MIN, SHADOW_OFFSET_MAX)
}

pub fn text_size(value: i32) -> i32 {
    value.clamp(TEXT_SIZE_MIN, TEXT_SIZE_MAX)
}

pub fn outline_size(value: i32) -> i32 {
    value.clamp(OUTLINE_SIZE_MIN, OUTLINE_SIZE_MAX)
}

pub fn eztrans_process_count(value: u32) -> u32 {
    value.clamp(EZTRANS_PROCESS_COUNT_MIN, EZTRANS_PROCESS_COUNT_MAX)
}

pub fn eztrans_process_count_usize(value: usize) -> usize {
    value.clamp(
        EZTRANS_PROCESS_COUNT_MIN as usize,
        EZTRANS_PROCESS_COUNT_MAX as usize,
    )
}

pub fn llm_max_tokens(value: u32) -> u32 {
    value.clamp(LLM_MAX_TOKENS_MIN, LLM_MAX_TOKENS_MAX)
}

pub fn llm_debounce_ms(value: u32) -> u32 {
    value.clamp(LLM_DEBOUNCE_MS_MIN, LLM_DEBOUNCE_MS_MAX)
}

pub fn llm_temperature(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(LLM_TEMPERATURE_MIN, LLM_TEMPERATURE_MAX)
    } else {
        LLM_TEMPERATURE_DEFAULT
    }
}

pub fn llm_temperature_slider(value: i32) -> f32 {
    llm_temperature(value as f32 / 100.0)
}

pub fn llm_temperature_to_slider(value: f32) -> i32 {
    (llm_temperature(value) * 100.0).round() as i32
}

pub fn llm_top_p(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(LLM_TOP_P_MIN, LLM_TOP_P_MAX)
    } else {
        LLM_TOP_P_DEFAULT
    }
}

pub fn llm_penalty(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(LLM_PENALTY_MIN, LLM_PENALTY_MAX)
    } else {
        LLM_PENALTY_DEFAULT
    }
}
