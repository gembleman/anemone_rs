//! 설정 대화상자
//!
//! 탭 기반 설정 대화상자. 외관/표시·윈도우/번역 3개 탭으로 분리.
//! Win32 SysTabControl32를 사용하여 탭 전환 시 컨트롤을 표시/숨김.

pub(self) mod ctrl_id;
mod handlers;

use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
        UI::Controls::*, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::config::{Config, TextAlign};
use crate::constants::{TBM_GETPOS_VAL, TCM_GETCURSEL, TCN_SELCHANGE};
use crate::impl_dialog;
use super::helpers::DialogControls;

/// 설정 변경 콜백 타입
pub type SettingsChangeCallback = Box<dyn Fn(&Config)>;

/// 탭 인덱스
const TAB_APPEARANCE: usize = 0;
const TAB_DISPLAY: usize = 1;
const TAB_TRANSLATION: usize = 2;

/// 설정 대화상자
pub struct SettingsDialog {
    hwnd: HWND,
    config: Rc<RefCell<Config>>,
    main_hwnd: HWND,
    on_change: Option<SettingsChangeCallback>,
    /// 각 탭에 속한 컨트롤 HWND 목록 (탭 전환 시 표시/숨김)
    tab_controls: [Vec<HWND>; 3],
    current_tab: usize,
}

impl DialogControls for SettingsDialog {
    fn dialog_hwnd(&self) -> HWND { self.hwnd }
}

impl_dialog! {
    dialog: SettingsDialog,
    instance: SETTINGS_INSTANCE,
    class_name: w!("AnemoneSettingsClass"),
    title: w!("아네모네 설정"),
    width: 500,
    height: 780,
    extra_style: WINDOW_STYLE::default(),
    params: (parent: HWND, config: Rc<RefCell<Config>>, on_change: Option<SettingsChangeCallback>),
    init: |hwnd, parent, config, on_change| {
        SettingsDialog {
            hwnd, config, main_hwnd: parent, on_change,
            tab_controls: [Vec::new(), Vec::new(), Vec::new()],
            current_tab: TAB_APPEARANCE,
        }
    },
}

impl SettingsDialog {
    // create_color_button은 create_button과 동일하지만 의미적으로 구분
    unsafe fn create_color_button(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str) -> Result<HWND> {
        unsafe { self.create_button(x, y, w, h, id, text) }
    }

    /// 컨트롤을 생성하고 지정된 탭에 등록
    fn register_control(&mut self, tab: usize, hwnd: HWND) {
        self.tab_controls[tab].push(hwnd);
    }

    /// 탭 전환: 현재 탭 컨트롤 숨기고 새 탭 컨트롤 표시
    fn switch_tab(&mut self, new_tab: usize) {
        if new_tab == self.current_tab || new_tab >= self.tab_controls.len() {
            return;
        }
        // SAFETY: All HWNDs in tab_controls are valid child window handles.
        unsafe {
            for &hwnd in &self.tab_controls[self.current_tab] {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            for &hwnd in &self.tab_controls[new_tab] {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
        }
        self.current_tab = new_tab;
    }

    /// 컨트롤 생성
    fn create_controls(&mut self) -> Result<()> {
        // SAFETY: self.hwnd is a valid window handle from show_impl. All control creation
        // methods use valid parent handle. SendMessageW calls use valid control handles.
        unsafe {
            let _hinst = GetModuleHandleW(None)?;
            let _hfont = GetStockObject(DEFAULT_GUI_FONT);

            // ====== 탭 컨트롤 ======
            let tabs = ["외관", "표시·윈도우", "번역"];
            self.create_tab_control(5, 5, 485, 710, ctrl_id::TAB_CONTROL, &tabs)?;

            // 탭 내부 컨트롤 시작 오프셋 (탭 헤더 아래)
            let tx = 15;  // 탭 영역 내부 x
            let ty = 35;  // 탭 헤더 높이 이후 y

            // ════════════════════════════════════════════
            // 탭 0: 외관 설정
            // ════════════════════════════════════════════
            self.create_tab0_appearance(tx, ty)?;

            // ════════════════════════════════════════════
            // 탭 1: 표시·윈도우
            // ════════════════════════════════════════════
            self.create_tab1_display(tx, ty)?;

            // ════════════════════════════════════════════
            // 탭 2: 번역 설정
            // ════════════════════════════════════════════
            self.create_tab2_translation(tx, ty)?;

            // 초기 상태: 탭 0만 표시, 나머지 숨김
            for &hwnd in &self.tab_controls[TAB_DISPLAY] {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            for &hwnd in &self.tab_controls[TAB_TRANSLATION] {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }

            // ====== 닫기 버튼 (탭 외부, 항상 표시) ======
            self.create_button(385, 722, 100, 30, ctrl_id::CLOSE, "닫기")?;

            Ok(())
        }
    }

    /// 탭 0: 외관 설정
    fn create_tab0_appearance(&mut self, tx: i32, ty: i32) -> Result<()> {
        unsafe {
            let tab = TAB_APPEARANCE;

            // ── 배경 설정 ──
            let h = self.create_group_box(tx, ty, 220, 75, "배경 설정")?;
            self.register_control(tab, h);

            let h = self.create_label(tx + 10, ty + 20, 50, 18, "투명도:")?;
            self.register_control(tab, h);
            let trackbar = self.create_trackbar(tx + 60, ty + 18, 150, 22, ctrl_id::BACKGROUND_TRACKBAR, 0, 255)?;
            self.register_control(tab, trackbar);
            let alpha = (self.config.borrow().background_color >> 24) & 0xFF;
            let _ = SendMessageW(trackbar, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(alpha as isize)));

            let h = self.create_color_button(tx + 10, ty + 45, 70, 24, ctrl_id::BACKGROUND_COLOR, "배경색")?;
            self.register_control(tab, h);
            let bg_visible = self.config.borrow().background_visible;
            let h = self.create_checkbox(tx + 90, ty + 47, 80, 20, ctrl_id::BACKGROUND_SWITCH, "표시", bg_visible)?;
            self.register_control(tab, h);

            // ── 텍스트 크기 ──
            let h = self.create_group_box(tx + 230, ty, 230, 75, "텍스트 크기")?;
            self.register_control(tab, h);

            let size_trackbar = self.create_trackbar(tx + 240, ty + 18, 210, 22, ctrl_id::TEXTSIZE_TRACKBAR, 6, 100)?;
            self.register_control(tab, size_trackbar);
            let text_size = self.config.borrow().translation_style.size;
            let _ = SendMessageW(size_trackbar, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(text_size as isize)));

            let h = self.create_button(tx + 240, ty + 45, 35, 24, ctrl_id::TEXTSIZE_MINUS, "-")?;
            self.register_control(tab, h);
            let h = self.create_button(tx + 280, ty + 45, 35, 24, ctrl_id::TEXTSIZE_PLUS, "+")?;
            self.register_control(tab, h);
            let h = self.create_label_with_id(tx + 325, ty + 48, 100, 18, ctrl_id::TEXTSIZE_TEXT, &format!("크기: {}", text_size))?;
            self.register_control(tab, h);

            // ── 외곽선 설정 ──
            let oy = ty + 80;
            let h = self.create_group_box(tx, oy, 460, 75, "외곽선 설정")?;
            self.register_control(tab, h);

            let h = self.create_label(tx + 10, oy + 18, 55, 18, "외곽선1:")?;
            self.register_control(tab, h);
            let outline1_tb = self.create_trackbar(tx + 65, oy + 16, 100, 22, ctrl_id::OUTLINE1_TRACKBAR, 0, 20)?;
            self.register_control(tab, outline1_tb);
            let outline1_size = self.config.borrow().translation_style.outline1_size;
            let _ = SendMessageW(outline1_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(outline1_size as isize)));
            let h = self.create_button(tx + 170, oy + 16, 25, 22, ctrl_id::OUTLINE1_MINUS, "-")?;
            self.register_control(tab, h);
            let h = self.create_button(tx + 198, oy + 16, 25, 22, ctrl_id::OUTLINE1_PLUS, "+")?;
            self.register_control(tab, h);

            let h = self.create_label(tx + 240, oy + 18, 55, 18, "외곽선2:")?;
            self.register_control(tab, h);
            let outline2_tb = self.create_trackbar(tx + 295, oy + 16, 100, 22, ctrl_id::OUTLINE2_TRACKBAR, 0, 20)?;
            self.register_control(tab, outline2_tb);
            let outline2_size = self.config.borrow().translation_style.outline2_size;
            let _ = SendMessageW(outline2_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(outline2_size as isize)));
            let h = self.create_button(tx + 400, oy + 16, 25, 22, ctrl_id::OUTLINE2_MINUS, "-")?;
            self.register_control(tab, h);
            let h = self.create_button(tx + 428, oy + 16, 25, 22, ctrl_id::OUTLINE2_PLUS, "+")?;
            self.register_control(tab, h);

            let h = self.create_label(tx + 10, oy + 46, 65, 18, "그림자 X:")?;
            self.register_control(tab, h);
            let shadow_x_tb = self.create_trackbar(tx + 75, oy + 44, 100, 22, ctrl_id::SHADOW_X_TRACKBAR, 0, 20)?;
            self.register_control(tab, shadow_x_tb);
            let shadow_x = self.config.borrow().shadow_offset_x;
            let _ = SendMessageW(shadow_x_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(shadow_x as isize)));

            let h = self.create_label(tx + 240, oy + 46, 65, 18, "그림자 Y:")?;
            self.register_control(tab, h);
            let shadow_y_tb = self.create_trackbar(tx + 305, oy + 44, 100, 22, ctrl_id::SHADOW_Y_TRACKBAR, 0, 20)?;
            self.register_control(tab, shadow_y_tb);
            let shadow_y = self.config.borrow().shadow_offset_y;
            let _ = SendMessageW(shadow_y_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(shadow_y as isize)));

            // ── 이름/원문/번역문 색상 설정 ──
            let cy = ty + 160;
            let name_shadow = self.config.borrow().name_style.shadow_enabled;
            self.create_text_style_group(tab, tx, cy, 150, "이름 설정",
                ctrl_id::NAME_COLOR, ctrl_id::NAME_OUTLINE1, ctrl_id::NAME_OUTLINE2,
                ctrl_id::NAME_SHADOW_COLOR, ctrl_id::NAME_FONT, ctrl_id::NAME_SHADOW,
                name_shadow)?;

            let org_shadow = self.config.borrow().original_style.shadow_enabled;
            self.create_text_style_group(tab, tx + 155, cy, 150, "원문 설정",
                ctrl_id::ORG_COLOR, ctrl_id::ORG_OUTLINE1, ctrl_id::ORG_OUTLINE2,
                ctrl_id::ORG_SHADOW_COLOR, ctrl_id::ORG_FONT, ctrl_id::ORG_SHADOW,
                org_shadow)?;

            let trans_shadow = self.config.borrow().translation_style.shadow_enabled;
            self.create_text_style_group(tab, tx + 310, cy, 150, "번역문 설정",
                ctrl_id::TRANS_COLOR, ctrl_id::TRANS_OUTLINE1, ctrl_id::TRANS_OUTLINE2,
                ctrl_id::TRANS_SHADOW_COLOR, ctrl_id::TRANS_FONT, ctrl_id::TRANS_SHADOW,
                trans_shadow)?;

            // ── 텍스트 여백 ──
            let my = ty + 290;
            let h = self.create_group_box(tx, my, 460, 50, "텍스트 여백")?;
            self.register_control(tab, h);

            let h = self.create_label(tx + 10, my + 22, 35, 18, "좌우:")?;
            self.register_control(tab, h);
            let margin_x_tb = self.create_trackbar(tx + 45, my + 20, 90, 22, ctrl_id::MARGIN_X_TRACKBAR, 0, 300)?;
            self.register_control(tab, margin_x_tb);
            let margin_x = self.config.borrow().text_margin_x;
            let _ = SendMessageW(margin_x_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(margin_x as isize)));

            let h = self.create_label(tx + 155, my + 22, 35, 18, "상하:")?;
            self.register_control(tab, h);
            let margin_y_tb = self.create_trackbar(tx + 190, my + 20, 90, 22, ctrl_id::MARGIN_Y_TRACKBAR, 0, 300)?;
            self.register_control(tab, margin_y_tb);
            let margin_y = self.config.borrow().text_margin_y;
            let _ = SendMessageW(margin_y_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(margin_y as isize)));

            let h = self.create_label(tx + 310, my + 22, 35, 18, "이름:")?;
            self.register_control(tab, h);
            let margin_name_tb = self.create_trackbar(tx + 345, my + 20, 90, 22, ctrl_id::MARGIN_NAME_TRACKBAR, 0, 300)?;
            self.register_control(tab, margin_name_tb);
            let margin_name = self.config.borrow().name_margin;
            let _ = SendMessageW(margin_name_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(margin_name as isize)));

            // ── 테두리 설정 ──
            let by = ty + 345;
            let h = self.create_group_box(tx, by, 460, 50, "테두리 설정")?;
            self.register_control(tab, h);
            let border_visible = self.config.borrow().border_visible;
            let h = self.create_checkbox(tx + 10, by + 22, 60, 20, ctrl_id::BORDER_MODE, "표시", border_visible)?;
            self.register_control(tab, h);
            let h = self.create_color_button(tx + 75, by + 20, 80, 24, ctrl_id::BORDER_COLOR, "테두리색")?;
            self.register_control(tab, h);
            let h = self.create_label(tx + 165, by + 22, 40, 18, "두께:")?;
            self.register_control(tab, h);
            let border_tb = self.create_trackbar(tx + 205, by + 20, 150, 22, ctrl_id::BORDER_SIZE_TRACKBAR, 0, 10)?;
            self.register_control(tab, border_tb);
            let border_size = self.config.borrow().border_width;
            let _ = SendMessageW(border_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(border_size as isize)));

            Ok(())
        }
    }

    /// 텍스트 스타일 그룹 생성 (이름/원문/번역문 공통)
    fn create_text_style_group(
        &mut self, tab: usize, x: i32, y: i32, w: i32, title: &str,
        color_id: u16, outline1_id: u16, outline2_id: u16,
        shadow_id: u16, font_id: u16, shadow_check_id: u16,
        shadow_enabled: bool,
    ) -> Result<()> {
        unsafe {
            let bw = (w - 20) / 2;  // 버튼 너비

            let h = self.create_group_box(x, y, w, 125, title)?;
            self.register_control(tab, h);

            let h = self.create_color_button(x + 5, y + 20, bw, 22, color_id, "주색상")?;
            self.register_control(tab, h);
            let h = self.create_color_button(x + 5 + bw + 5, y + 20, bw, 22, outline1_id, "외곽1")?;
            self.register_control(tab, h);
            let h = self.create_color_button(x + 5, y + 47, bw, 22, outline2_id, "외곽2")?;
            self.register_control(tab, h);
            let h = self.create_color_button(x + 5 + bw + 5, y + 47, bw, 22, shadow_id, "그림자")?;
            self.register_control(tab, h);
            let h = self.create_button(x + 5, y + 74, w - 10, 22, font_id, "폰트 선택")?;
            self.register_control(tab, h);
            let h = self.create_checkbox(x + 5, y + 100, 100, 20, shadow_check_id, "그림자", shadow_enabled)?;
            self.register_control(tab, h);

            Ok(())
        }
    }

    /// 탭 1: 표시·윈도우 옵션
    fn create_tab1_display(&mut self, tx: i32, ty: i32) -> Result<()> {
        unsafe {
            let tab = TAB_DISPLAY;

            // ── 표시 옵션 ──
            let h = self.create_group_box(tx, ty, 460, 100, "표시 옵션")?;
            self.register_control(tab, h);

            let show_org = self.config.borrow().show_original;
            let h = self.create_checkbox(tx + 15, ty + 25, 100, 20, ctrl_id::PRINT_ORGTEXT, "원문 표시", show_org)?;
            self.register_control(tab, h);
            let show_trans = self.config.borrow().show_translation;
            let h = self.create_checkbox(tx + 130, ty + 25, 100, 20, ctrl_id::PRINT_TRANSTEXT, "번역 표시", show_trans)?;
            self.register_control(tab, h);
            let show_name = self.config.borrow().show_name;
            let h = self.create_checkbox(tx + 245, ty + 25, 100, 20, ctrl_id::PRINT_ORGNAME, "이름 표시", show_name)?;
            self.register_control(tab, h);
            let sep_name = self.config.borrow().separate_name;
            let h = self.create_checkbox(tx + 360, ty + 25, 100, 20, ctrl_id::SEPERATE_NAME, "이름 분리", sep_name)?;
            self.register_control(tab, h);

            let repeat_mode = self.config.borrow().repeat_text_mode;
            let h = self.create_button(tx + 15, ty + 55, 65, 24, ctrl_id::REPEAT_TEXT, &format!("반복:{}", repeat_mode))?;
            self.register_control(tab, h);

            let align = self.config.borrow().text_align;
            let h = self.create_radio(tx + 100, ty + 58, 55, 20, ctrl_id::TEXTALIGN_LEFT, "왼쪽", align == TextAlign::Left)?;
            self.register_control(tab, h);
            let h = self.create_radio(tx + 165, ty + 58, 55, 20, ctrl_id::TEXTALIGN_MID, "중앙", align == TextAlign::Center)?;
            self.register_control(tab, h);
            let h = self.create_radio(tx + 230, ty + 58, 55, 20, ctrl_id::TEXTALIGN_RIGHT, "오른쪽", align == TextAlign::Right)?;
            self.register_control(tab, h);

            // ── 윈도우 옵션 ──
            let wy = ty + 110;
            let h = self.create_group_box(tx, wy, 460, 80, "윈도우 옵션")?;
            self.register_control(tab, h);

            let topmost = self.config.borrow().window_topmost;
            let h = self.create_checkbox(tx + 15, wy + 25, 80, 20, ctrl_id::TOPMOST, "항상 위", topmost)?;
            self.register_control(tab, h);
            let magnetic = self.config.borrow().magnetic_mode;
            let h = self.create_checkbox(tx + 110, wy + 25, 70, 20, ctrl_id::USE_MAGNETIC, "자석", magnetic)?;
            self.register_control(tab, h);
            let magnetic_min = self.config.borrow().magnetic_minimize;
            let h = self.create_checkbox(tx + 195, wy + 25, 70, 20, ctrl_id::MAGNETIC_MINIMIZE, "최소", magnetic_min)?;
            self.register_control(tab, h);

            let hide_win = self.config.borrow().temp_window_hide;
            let h = self.create_checkbox(tx + 15, wy + 50, 80, 20, ctrl_id::HIDEWIN, "숨기기", hide_win)?;
            self.register_control(tab, h);
            let clip_watch = self.config.borrow().clipboard_watch;
            let h = self.create_checkbox(tx + 110, wy + 50, 80, 20, ctrl_id::CLIPBOARD_WATCH, "클립보드", clip_watch)?;
            self.register_control(tab, h);
            let click_through = self.config.borrow().click_through;
            let h = self.create_checkbox(tx + 205, wy + 50, 90, 20, ctrl_id::WNDCLICK_THROUGH, "클릭 통과", click_through)?;
            self.register_control(tab, h);

            Ok(())
        }
    }

    /// 탭 2: 번역 설정
    fn create_tab2_translation(&mut self, tx: i32, ty: i32) -> Result<()> {
        unsafe {
            let tab = TAB_TRANSLATION;

            // ── 공통 엔진/언어 ──
            let h = self.create_group_box(tx, ty, 460, 130, "번역 설정")?;
            self.register_control(tab, h);

            // 엔진 선택
            let h = self.create_label(tx + 15, ty + 25, 40, 18, "엔진:")?;
            self.register_control(tab, h);
            let engine_items = vec!["EzTrans", "Google", "DeepL", "Papago", "LLM"];
            let engine_sel = self.config.borrow().translation.engine_as_u8() as usize;
            let h = self.create_combobox(tx + 55, ty + 23, 90, 120, ctrl_id::TRANS_ENGINE, &engine_items, engine_sel)?;
            self.register_control(tab, h);

            // 소스/타겟 언어
            let h = self.create_label(tx + 160, ty + 25, 40, 18, "소스:")?;
            self.register_control(tab, h);
            let lang_items = vec!["일본어", "한국어", "영어", "중국어"];
            let config = self.config.borrow();
            let engine = config.translation.get_engine();
            let source_sel = config.translation.source_lang_index(engine);
            drop(config);
            let h = self.create_combobox(tx + 200, ty + 23, 80, 100, ctrl_id::TRANS_SOURCE_LANG, &lang_items, source_sel)?;
            self.register_control(tab, h);

            let h = self.create_label(tx + 295, ty + 25, 40, 18, "타겟:")?;
            self.register_control(tab, h);
            let config = self.config.borrow();
            let target_sel = config.translation.target_lang_index(engine);
            drop(config);
            let h = self.create_combobox(tx + 335, ty + 23, 80, 100, ctrl_id::TRANS_TARGET_LANG, &lang_items, target_sel)?;
            self.register_control(tab, h);

            let auto_detect = self.config.borrow().translation.auto_detect;
            let h = self.create_checkbox(tx + 420, ty + 25, 55, 18, ctrl_id::TRANS_AUTO_DETECT, "자동", auto_detect)?;
            self.register_control(tab, h);

            // EzTrans 경로
            let h = self.create_label(tx + 15, ty + 60, 80, 18, "EzTrans DLL:")?;
            self.register_control(tab, h);
            let dll_path = self.config.borrow().translation.eztrans_dll_path.clone();
            let h = self.create_edit(tx + 95, ty + 58, 280, 22, ctrl_id::EZTRANS_DLL_EDIT, &dll_path)?;
            self.register_control(tab, h);
            let h = self.create_button(tx + 380, ty + 58, 70, 22, ctrl_id::EZTRANS_DLL_BROWSE, "찾아보기")?;
            self.register_control(tab, h);

            let h = self.create_label(tx + 15, ty + 90, 80, 18, "EzTrans Dat:")?;
            self.register_control(tab, h);
            let dat_path = self.config.borrow().translation.eztrans_dat_path.clone();
            let h = self.create_edit(tx + 95, ty + 88, 280, 22, ctrl_id::EZTRANS_DAT_EDIT, &dat_path)?;
            self.register_control(tab, h);
            let h = self.create_button(tx + 380, ty + 88, 70, 22, ctrl_id::EZTRANS_DAT_BROWSE, "찾아보기")?;
            self.register_control(tab, h);

            // ── DeepL 그룹 ──
            let dy = ty + 140;
            let h = self.create_group_box(tx, dy, 460, 135, "DeepL")?;
            self.register_control(tab, h);

            // 단일 키 (deepl_keys 비어있을 때만 사용)
            let h = self.create_label(tx + 15, dy + 25, 80, 18, "API 키 (단일):")?;
            self.register_control(tab, h);
            let api_key = self.config.borrow().translation.deepl_api_key.clone();
            let h = self.create_edit(tx + 100, dy + 23, 345, 22, ctrl_id::DEEPL_API_KEY_EDIT, &api_key)?;
            self.register_control(tab, h);

            // 보조 키 목록 (멀티 키)
            let h = self.create_label(tx + 15, dy + 53, 100, 18, "보조 키 목록:")?;
            self.register_control(tab, h);
            let h = self.create_label(tx + 305, dy + 53, 50, 18, "전략:")?;
            self.register_control(tab, h);
            let strategy_items = vec!["failover", "round-robin"];
            let strategy_sel: usize = match self.config.borrow().translation.deepl_strategy.to_lowercase().as_str() {
                "round-robin" | "roundrobin" | "rr" => 1,
                _ => 0,
            };
            let h = self.create_combobox(tx + 345, dy + 51, 100, 120, ctrl_id::DEEPL_STRATEGY_COMBO, &strategy_items, strategy_sel)?;
            self.register_control(tab, h);

            let h = self.create_listbox(tx + 15, dy + 75, 285, 50, ctrl_id::DEEPL_KEYS_LIST)?;
            self.register_control(tab, h);
            // 초기 항목 채우기
            {
                let cfg = self.config.borrow();
                use crate::constants::LB_ADDSTRING;
                use crate::util::to_wide;
                for k in &cfg.translation.deepl_keys {
                    let kw = to_wide(k);
                    let _ = SendMessageW(h, LB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(kw.as_ptr() as isize)));
                }
            }

            let h = self.create_edit(tx + 305, dy + 75, 140, 22, ctrl_id::DEEPL_KEY_ADD_EDIT, "")?;
            self.register_control(tab, h);
            let h = self.create_button(tx + 305, dy + 100, 65, 22, ctrl_id::DEEPL_KEY_ADD_BTN, "추가")?;
            self.register_control(tab, h);
            let h = self.create_button(tx + 380, dy + 100, 65, 22, ctrl_id::DEEPL_KEY_REMOVE_BTN, "삭제")?;
            self.register_control(tab, h);

            // ── Papago 그룹 ──
            let py = ty + 285;
            let h = self.create_group_box(tx, py, 460, 70, "Papago")?;
            self.register_control(tab, h);

            let h = self.create_label(tx + 15, py + 22, 80, 18, "Client ID:")?;
            self.register_control(tab, h);
            let papago_id = self.config.borrow().translation.papago_client_id.clone();
            let h = self.create_edit(tx + 100, py + 20, 345, 22, ctrl_id::PAPAGO_ID_EDIT, &papago_id)?;
            self.register_control(tab, h);

            let h = self.create_label(tx + 15, py + 47, 80, 18, "Secret:")?;
            self.register_control(tab, h);
            let papago_secret = self.config.borrow().translation.papago_client_secret.clone();
            let h = self.create_edit(tx + 100, py + 45, 345, 22, ctrl_id::PAPAGO_SECRET_EDIT, &papago_secret)?;
            self.register_control(tab, h);

            // ── LLM 번역 그룹 ──
            let ly = ty + 365;
            let h = self.create_group_box(tx, ly, 460, 305, "LLM 번역")?;
            self.register_control(tab, h);

            // 제공자 + 모델
            let h = self.create_label(tx + 15, ly + 25, 50, 18, "제공자:")?;
            self.register_control(tab, h);
            let provider_items = vec!["OpenAI", "Anthropic", "Gemini", "Grok", "OpenRouter"];
            let provider_sel = self.config.borrow().translation.llm.get_provider() as u8 as usize;
            let h = self.create_combobox(tx + 65, ly + 23, 100, 150, ctrl_id::LLM_PROVIDER, &provider_items, provider_sel)?;
            self.register_control(tab, h);

            let h = self.create_label(tx + 180, ly + 25, 40, 18, "모델:")?;
            self.register_control(tab, h);
            let model = self.config.borrow().translation.llm.model.clone();
            let h = self.create_edit(tx + 220, ly + 23, 230, 22, ctrl_id::LLM_MODEL_EDIT, &model)?;
            self.register_control(tab, h);

            // API 키
            let h = self.create_label(tx + 15, ly + 55, 50, 18, "API 키:")?;
            self.register_control(tab, h);
            let api_key = self.config.borrow().translation.llm.api_key.clone();
            let h = self.create_edit(tx + 65, ly + 53, 385, 22, ctrl_id::LLM_API_KEY_EDIT, &api_key)?;
            self.register_control(tab, h);

            // Base URL
            let h = self.create_label(tx + 15, ly + 85, 60, 18, "Base URL:")?;
            self.register_control(tab, h);
            let base_url = self.config.borrow().translation.llm.base_url.clone();
            let h = self.create_edit(tx + 80, ly + 83, 370, 22, ctrl_id::LLM_BASE_URL_EDIT, &base_url)?;
            self.register_control(tab, h);

            // 시스템 프롬프트 (멀티라인)
            let h = self.create_label(tx + 15, ly + 115, 100, 18, "시스템 프롬프트:")?;
            self.register_control(tab, h);
            let system_prompt = self.config.borrow().translation.llm.system_prompt.clone();
            let h = self.create_multiline_edit(tx + 15, ly + 135, 435, 60, ctrl_id::LLM_SYSTEM_PROMPT_EDIT, &system_prompt)?;
            self.register_control(tab, h);

            // Temperature 트랙바 (0.00 ~ 2.00, 100 단위 = 0~200)
            let temperature_value = self.config.borrow().translation.llm.temperature;
            let h = self.create_label(tx + 15, ly + 205, 90, 18, "Temperature:")?;
            self.register_control(tab, h);
            let temp_tb = self.create_trackbar(tx + 105, ly + 203, 180, 22, ctrl_id::LLM_TEMPERATURE_TRACKBAR, 0, 200)?;
            self.register_control(tab, temp_tb);
            let temp_pos = (temperature_value * 100.0).clamp(0.0, 200.0) as isize;
            let _ = SendMessageW(temp_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(temp_pos)));
            let h = self.create_label_with_id(tx + 290, ly + 207, 70, 18, ctrl_id::LLM_TEMPERATURE_LABEL, &format!("{:.2}", temperature_value))?;
            self.register_control(tab, h);

            // Max Tokens
            let h = self.create_label(tx + 365, ly + 207, 30, 18, "Max:")?;
            self.register_control(tab, h);
            let max_tokens = format!("{}", self.config.borrow().translation.llm.max_tokens);
            let h = self.create_edit(tx + 395, ly + 205, 55, 22, ctrl_id::LLM_MAX_TOKENS_EDIT, &max_tokens)?;
            self.register_control(tab, h);

            // Debounce + Glossary 편집
            let h = self.create_label(tx + 15, ly + 237, 90, 18, "디바운스(ms):")?;
            self.register_control(tab, h);
            let debounce = format!("{}", self.config.borrow().translation.llm.debounce_ms);
            let h = self.create_edit(tx + 105, ly + 235, 70, 22, ctrl_id::LLM_DEBOUNCE_EDIT, &debounce)?;
            self.register_control(tab, h);

            let glossary_count = self.config.borrow().translation.llm.glossary.len();
            let h = self.create_label_with_id(tx + 195, ly + 237, 120, 18, ctrl_id::LLM_GLOSSARY_COUNT_LABEL, &format!("사전 항목: {}", glossary_count))?;
            self.register_control(tab, h);
            let h = self.create_button(tx + 325, ly + 235, 125, 22, ctrl_id::LLM_GLOSSARY_EDIT_BTN, "사전 편집...")?;
            self.register_control(tab, h);

            Ok(())
        }
    }

    /// 커스텀 메시지 핸들러
    fn handle_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_HSCROLL => {
                // SAFETY: lparam contains a valid trackbar HWND from the system.
                unsafe {
                    let code = (wparam.0 & 0xFFFF) as u32;
                    let trackbar_hwnd = HWND(lparam.0 as *mut _);

                    let id = GetDlgCtrlID(trackbar_hwnd) as u16;
                    let value = match code {
                        TB_THUMBTRACK => ((wparam.0 >> 16) & 0xFFFF) as i32,
                        TB_LINEUP | TB_LINEDOWN | TB_PAGEUP | TB_PAGEDOWN | TB_TOP
                        | TB_BOTTOM | TB_ENDTRACK => {
                            SendMessageW(
                                trackbar_hwnd,
                                TBM_GETPOS_VAL,
                                Some(WPARAM(0)),
                                Some(LPARAM(0)),
                            ).0 as i32
                        }
                        _ => return Some(LRESULT(0)),
                    };

                    self.handle_trackbar(id, value);
                }
                Some(LRESULT(0))
            }
            WM_NOTIFY => {
                // SAFETY: lparam points to a valid NMHDR struct from the system.
                unsafe {
                    let nmhdr = &*(lparam.0 as *const NMHDR);
                    if nmhdr.code == TCN_SELCHANGE as u32 {
                        if let Ok(tab_hwnd) = GetDlgItem(Some(self.hwnd), ctrl_id::TAB_CONTROL as i32) {
                            let sel = SendMessageW(
                                tab_hwnd,
                                TCM_GETCURSEL,
                                Some(WPARAM(0)),
                                Some(LPARAM(0)),
                            ).0 as usize;
                            self.switch_tab(sel);
                        }
                    }
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}
