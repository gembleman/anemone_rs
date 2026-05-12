//! 설정 대화상자
//!
//! 탭 기반 설정 대화상자. 외관/표시·윈도우/번역 3개 탭으로 분리.
//! Win32 SysTabControl32를 사용하여 탭 전환 시 컨트롤을 표시/숨김.

mod ctrl_id;
mod handlers;

use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
        UI::Controls::*, UI::Input::KeyboardAndMouse::EnableWindow, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use crate::config::{Config, TextAlign};
use crate::constants::TBM_GETPOS_VAL;
use crate::define_dialog_instance;
use crate::translation::{TranslationEngine, lang_utils};
use crate::util::to_wide;
use super::helpers::{Dialog, DialogControls};

/// 설정 변경 콜백 타입
pub type SettingsChangeCallback = Box<dyn Fn(&Config)>;

/// 탭 인덱스
const TAB_APPEARANCE: usize = 0;
const TAB_DISPLAY: usize = 1;
const TAB_TRANSLATION: usize = 2;

/// 텍스트 스타일 그룹 6종 컨트롤 ID + 그림자 초기 상태 (이름/원문/번역문 공통 구조)
#[derive(Clone, Copy)]
struct TextStyleGroupSpec {
    color: u16,
    outline1: u16,
    outline2: u16,
    shadow: u16,
    font: u16,
    shadow_check: u16,
    shadow_enabled: bool,
}

/// 엔진별 컨트롤 그룹 (활성/비활성 토글용)
#[derive(Clone, Copy)]
pub(super) enum EngineGroup {
    EzTrans = 0,
    DeepL = 1,
    Papago = 2,
    Llm = 3,
}

/// 반복 모드 라벨 (repeat_text_mode 값에 대응)
pub(super) fn repeat_mode_label(mode: u8) -> String {
    let name = match mode {
        0 => "끄기",
        1 => "한 번",
        2 => "계속",
        3 => "역순",
        4 => "랜덤",
        _ => "?",
    };
    format!("반복: {}", name)
}

/// 설정 대화상자
pub struct SettingsDialog {
    hwnd: HWND,
    config: Rc<RefCell<Config>>,
    main_hwnd: HWND,
    on_change: Option<SettingsChangeCallback>,
    /// 각 탭에 속한 컨트롤 HWND 목록 (탭 전환 시 표시/숨김)
    tab_controls: [Vec<HWND>; 3],
    current_tab: usize,
    /// 엔진별 컨트롤 (EnableWindow 토글용)
    pub(super) engine_controls: [Vec<HWND>; 4],
}

impl DialogControls for SettingsDialog {
    fn dialog_hwnd(&self) -> HWND { self.hwnd }
}

define_dialog_instance!(SETTINGS_INSTANCE: SettingsDialog);

impl Dialog for SettingsDialog {
    type Params = (Rc<RefCell<Config>>, Option<SettingsChangeCallback>);

    const CLASS_NAME: PCWSTR = w!("AnemoneSettingsClass");
    const TITLE: PCWSTR = w!("아네모네 설정");
    // 클라이언트 폭. 콘텐츠(그룹박스) 우측 끝 475 기준 좌우 여백 10/10.
    const WIDTH: i32 = 485;
    const HEIGHT: i32 = 780;
    const EXTRA_STYLE: WINDOW_STYLE = WINDOW_STYLE(0);

    fn instance_slot()
        -> &'static std::thread::LocalKey<
            std::cell::RefCell<Option<std::rc::Rc<std::cell::RefCell<Self>>>>,
        > {
        &SETTINGS_INSTANCE
    }

    fn init(hwnd: HWND, parent: HWND, params: Self::Params) -> Self {
        let (config, on_change) = params;
        SettingsDialog {
            hwnd, config, main_hwnd: parent, on_change,
            tab_controls: [Vec::new(), Vec::new(), Vec::new()],
            current_tab: TAB_APPEARANCE,
            engine_controls: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        }
    }

    fn create_controls(&mut self) -> Result<()> {
        SettingsDialog::create_controls(self)
    }

    fn handle_command(&mut self, cmd: u16, notify_code: u32) {
        SettingsDialog::handle_command(self, cmd, notify_code);
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        SettingsDialog::handle_message(self, msg, wparam, lparam)
    }
}

impl SettingsDialog {
    /// 기존 3-인자 시그니처를 유지하는 공개 진입점.
    pub fn show(
        parent: HWND,
        config: Rc<RefCell<Config>>,
        on_change: Option<SettingsChangeCallback>,
    ) -> Result<HWND> {
        <Self as Dialog>::show(parent, (config, on_change))
    }
}

impl SettingsDialog {
    /// 오너드로우 색상 버튼 생성 (BS_OWNERDRAW)
    unsafe fn create_color_button(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str) -> Result<HWND> {
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        // SAFETY: dialog hwnd is valid; CreateWindowExW creates a child button.
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let text_wide = crate::util::to_wide(text);
            let dpi = crate::dpi::dpi_for_window(self.hwnd);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(text_wide.as_ptr()),
                WINDOW_STYLE(
                    BS_OWNERDRAW as u32 | WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0,
                ),
                crate::dpi::scale(x, dpi),
                crate::dpi::scale(y, dpi),
                crate::dpi::scale(w, dpi),
                crate::dpi::scale(h, dpi),
                Some(self.hwnd),
                Some(HMENU(id as isize as *mut _)),
                Some(hinst.into()),
                None,
            )?;
            let hfont = super::helpers::dialog_font();
            let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));
            Ok(hwnd)
        }
    }

    /// ctrl_id로부터 미리보기 색상(ARGB)을 찾는다
    fn color_for_button(&self, id: u16) -> Option<u32> {
        use crate::config::{ColorType, TextType};
        let cfg = self.config.borrow();
        let argb = match id {
            ctrl_id::BACKGROUND_COLOR => cfg.background_color,
            ctrl_id::BORDER_COLOR => cfg.border_color,
            ctrl_id::NAME_COLOR => cfg.get_text_color(TextType::Name, ColorType::Primary),
            ctrl_id::NAME_OUTLINE1 => cfg.get_text_color(TextType::Name, ColorType::Outline1),
            ctrl_id::NAME_OUTLINE2 => cfg.get_text_color(TextType::Name, ColorType::Outline2),
            ctrl_id::NAME_SHADOW_COLOR => cfg.get_text_color(TextType::Name, ColorType::Shadow),
            ctrl_id::ORG_COLOR => cfg.get_text_color(TextType::Original, ColorType::Primary),
            ctrl_id::ORG_OUTLINE1 => cfg.get_text_color(TextType::Original, ColorType::Outline1),
            ctrl_id::ORG_OUTLINE2 => cfg.get_text_color(TextType::Original, ColorType::Outline2),
            ctrl_id::ORG_SHADOW_COLOR => cfg.get_text_color(TextType::Original, ColorType::Shadow),
            ctrl_id::TRANS_COLOR => cfg.get_text_color(TextType::Translation, ColorType::Primary),
            ctrl_id::TRANS_OUTLINE1 => cfg.get_text_color(TextType::Translation, ColorType::Outline1),
            ctrl_id::TRANS_OUTLINE2 => cfg.get_text_color(TextType::Translation, ColorType::Outline2),
            ctrl_id::TRANS_SHADOW_COLOR => cfg.get_text_color(TextType::Translation, ColorType::Shadow),
            _ => return None,
        };
        Some(argb)
    }

    /// WM_DRAWITEM 처리: 색상 버튼에 색상 스왓치 + 텍스트를 그린다
    fn draw_color_button(&self, dis: &DRAWITEMSTRUCT) {
        let id = dis.CtlID as u16;
        let Some(argb) = self.color_for_button(id) else { return; };
        // SAFETY: hDC and rcItem are valid for the duration of WM_DRAWITEM.
        unsafe {
            let hdc = dis.hDC;
            let rc = dis.rcItem;

            // 배경: 시스템 버튼 면색
            let bg_brush = GetSysColorBrush(COLOR_BTNFACE);
            let _ = FillRect(hdc, &rc, bg_brush);

            // 테두리 (눌림 / 포커스 상태에 따라 다르게)
            let pressed = (dis.itemState.0 & ODS_SELECTED.0) != 0;
            if pressed {
                let _ = DrawEdge(hdc, &mut { rc }, EDGE_SUNKEN, BF_RECT);
            } else {
                let _ = DrawEdge(hdc, &mut { rc }, EDGE_RAISED, BF_RECT);
            }

            // 색상 스왓치 (왼쪽 1/3 영역)
            let pad = 4;
            let swatch_w = ((rc.right - rc.left - pad * 3) / 3).max(12);
            let swatch_rc = RECT {
                left: rc.left + pad,
                top: rc.top + pad,
                right: rc.left + pad + swatch_w,
                bottom: rc.bottom - pad,
            };
            // ARGB → COLORREF (0x00BBGGRR)
            let r = (argb >> 16) & 0xFF;
            let g = (argb >> 8) & 0xFF;
            let b = argb & 0xFF;
            let colorref = COLORREF((b << 16) | (g << 8) | r);
            let brush = CreateSolidBrush(colorref);
            let _ = FillRect(hdc, &swatch_rc, brush);
            // 스왓치 외곽선
            let pen_color = COLORREF(0x00808080);
            let pen = CreatePen(PS_SOLID, 1, pen_color);
            let old_pen = SelectObject(hdc, pen.into());
            let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
            let _ = Rectangle(hdc, swatch_rc.left, swatch_rc.top, swatch_rc.right, swatch_rc.bottom);
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_brush);
            let _ = DeleteObject(pen.into());
            let _ = DeleteObject(brush.into());

            // 텍스트 (오른쪽 2/3 영역)
            let mut text_rc = RECT {
                left: swatch_rc.right + pad,
                top: rc.top,
                right: rc.right - pad,
                bottom: rc.bottom,
            };
            let mut buf = [0u16; 64];
            let len = GetWindowTextW(dis.hwndItem, &mut buf);
            if len > 0 {
                let _ = SetBkMode(hdc, TRANSPARENT);
                let _ = DrawTextW(hdc, &mut buf[..len as usize], &mut text_rc,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE);
            }

            // 포커스 사각형
            if (dis.itemState.0 & ODS_FOCUS.0) != 0 {
                let _ = DrawFocusRect(hdc, &rc);
            }
        }
    }

    /// 색상 변경 후 해당 버튼만 강제 다시 그리기
    pub(super) fn invalidate_color_button(&self, id: u16) {
        // SAFETY: self.hwnd is valid; GetDlgItem returns a valid control handle.
        unsafe {
            if let Ok(h) = GetDlgItem(Some(self.hwnd), id as i32) {
                let _ = InvalidateRect(Some(h), None, true);
            }
        }
    }

    /// 컨트롤을 생성하고 지정된 탭에 등록
    fn register_control(&mut self, tab: usize, hwnd: HWND) {
        self.tab_controls[tab].push(hwnd);
    }

    /// 엔진 그룹에 컨트롤 등록 (EnableWindow 토글용)
    fn register_engine_control(&mut self, group: EngineGroup, hwnd: HWND) {
        self.engine_controls[group as usize].push(hwnd);
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
        self.adjust_dialog_size_for_tab(new_tab);
    }

    /// 탭에 따라 다이얼로그 클라이언트 높이를 조정 (빈 공간 최소화)
    fn adjust_dialog_size_for_tab(&self, tab: usize) {
        // 디자인 클라이언트 높이. 닫기 버튼은 target_height - 65 에 배치되므로,
        // 각 탭 마지막 그룹박스 하단 아래로 닫기 버튼이 오도록 잡는다.
        let target_height = match tab {
            TAB_APPEARANCE => 505,
            TAB_DISPLAY => 305,
            TAB_TRANSLATION => 865,
            _ => 505,
        };
        // SAFETY: self.hwnd is valid. SetWindowPos uses valid parameters.
        unsafe {
            let dpi = crate::dpi::dpi_for_window(self.hwnd);
            let s = |v: i32| crate::dpi::scale(v, dpi);
            // 디자인 좌표는 클라이언트 기준. SetWindowPos 는 윈도우 전체 크기를
            // 받으므로 타이틀/테두리만큼 더해 환산해야 닫기 버튼이 안 잘린다.
            let (_, win_h) = super::helpers::design_to_window_size(
                self.hwnd, Self::WIDTH, target_height,
            );
            let mut rect = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut rect);
            let cur_w = rect.right - rect.left;
            let _ = SetWindowPos(
                self.hwnd, None,
                0, 0,
                cur_w, win_h,
                SWP_NOMOVE | SWP_NOZORDER,
            );
            // 탭 컨트롤도 같이 늘리기 (탭 헤더 ~ 닫기 버튼 위까지).
            // 폭은 create_controls 와 동일하게 클라 폭 - 좌우 5px = 475.
            if let Ok(tab_hwnd) = GetDlgItem(Some(self.hwnd), ctrl_id::TAB_CONTROL as i32) {
                let _ = SetWindowPos(
                    tab_hwnd, None,
                    0, 0,
                    s(475), s(target_height - 70),
                    SWP_NOMOVE | SWP_NOZORDER,
                );
            }
            // 닫기 버튼 재배치 (다이얼로그 하단). 클라 폭 485 → 우측 정렬 X=370.
            if let Ok(close_hwnd) = GetDlgItem(Some(self.hwnd), ctrl_id::CLOSE as i32) {
                let _ = SetWindowPos(
                    close_hwnd, None,
                    s(370), s(target_height - 65),
                    0, 0,
                    SWP_NOSIZE | SWP_NOZORDER,
                );
            }
        }
    }

    /// 엔진별 컨트롤 enable 상태 갱신
    fn update_engine_enable(&self, engine: TranslationEngine) {
        let active = match engine {
            TranslationEngine::EzTrans => EngineGroup::EzTrans as usize,
            TranslationEngine::DeepL => EngineGroup::DeepL as usize,
            TranslationEngine::Papago => EngineGroup::Papago as usize,
            TranslationEngine::Llm => EngineGroup::Llm as usize,
            // Google은 별도 입력란이 없으므로 전부 비활성
            TranslationEngine::Google => usize::MAX,
        };
        // SAFETY: HWNDs in engine_controls are valid child controls.
        unsafe {
            for (idx, group) in self.engine_controls.iter().enumerate() {
                let enable = idx == active;
                for &h in group {
                    let _ = EnableWindow(h, enable);
                }
            }
        }
    }

    /// 현재 엔진에 맞춰 소스/타겟 언어 콤보 항목을 갱신
    pub(super) fn refresh_language_combos(&self, engine: TranslationEngine) {
        // SAFETY: self.hwnd is valid; GetDlgItem returns valid combobox handles.
        unsafe {
            let src_combo = GetDlgItem(Some(self.hwnd), ctrl_id::TRANS_SOURCE_LANG as i32);
            let tgt_combo = GetDlgItem(Some(self.hwnd), ctrl_id::TRANS_TARGET_LANG as i32);
            let (Ok(src), Ok(tgt)) = (src_combo, tgt_combo) else { return; };

            let _ = SendMessageW(src, CB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
            for &lang in engine.supported_source_languages() {
                let w = to_wide(lang_utils::to_korean_name(lang));
                let _ = SendMessageW(src, CB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(w.as_ptr() as isize)));
            }
            let src_sel = self.config.borrow().translation.source_lang_index(engine);
            let _ = SendMessageW(src, CB_SETCURSEL, Some(WPARAM(src_sel)), Some(LPARAM(0)));

            let _ = SendMessageW(tgt, CB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
            for &lang in engine.supported_target_languages() {
                let w = to_wide(lang_utils::to_korean_name(lang));
                let _ = SendMessageW(tgt, CB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(w.as_ptr() as isize)));
            }
            let tgt_sel = self.config.borrow().translation.target_lang_index(engine);
            let _ = SendMessageW(tgt, CB_SETCURSEL, Some(WPARAM(tgt_sel)), Some(LPARAM(0)));
        }
    }

    /// 엔진별 enable 상태와 언어 콤보를 동시에 갱신 (엔진 변경 시 호출)
    pub(super) fn apply_engine_state(&self, engine: TranslationEngine) {
        self.update_engine_enable(engine);
        self.refresh_language_combos(engine);
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
            // 탭 높이는 switch_tab의 adjust_dialog_size_for_tab에서 동적 조정.
            // 폭은 클라 폭(485) - 좌5 - 우5 = 475 로 잡아 우측 여백 확보.
            self.create_tab_control(5, 5, 475, 400, ctrl_id::TAB_CONTROL, &tabs)?;

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
            // 위치는 adjust_dialog_size_for_tab에서 탭별로 재조정.
            // 클라 폭 485 기준 우측 정렬: 485 - 15 - 100 = 370.
            self.create_button(370, 405, 100, 30, ctrl_id::CLOSE, "닫기")?;

            // 초기 탭(외관)에 맞춰 다이얼로그 크기 조정
            self.adjust_dialog_size_for_tab(TAB_APPEARANCE);

            // 초기 엔진에 맞춰 활성/비활성 및 언어 콤보 적용
            let engine = self.config.borrow().translation.get_engine();
            self.apply_engine_state(engine);

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

            let h = self.create_label(tx + 10, oy + 18, 65, 18, "외곽선1:")?;
            self.register_control(tab, h);
            let outline1_tb = self.create_trackbar(tx + 75, oy + 16, 90, 22, ctrl_id::OUTLINE1_TRACKBAR, 0, 20)?;
            self.register_control(tab, outline1_tb);
            let outline1_size = self.config.borrow().translation_style.outline1_size;
            let _ = SendMessageW(outline1_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(outline1_size as isize)));
            let h = self.create_button(tx + 170, oy + 16, 25, 22, ctrl_id::OUTLINE1_MINUS, "-")?;
            self.register_control(tab, h);
            let h = self.create_button(tx + 198, oy + 16, 25, 22, ctrl_id::OUTLINE1_PLUS, "+")?;
            self.register_control(tab, h);

            let h = self.create_label(tx + 240, oy + 18, 65, 18, "외곽선2:")?;
            self.register_control(tab, h);
            let outline2_tb = self.create_trackbar(tx + 305, oy + 16, 90, 22, ctrl_id::OUTLINE2_TRACKBAR, 0, 20)?;
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
            let name_spec = TextStyleGroupSpec {
                color: ctrl_id::NAME_COLOR,
                outline1: ctrl_id::NAME_OUTLINE1,
                outline2: ctrl_id::NAME_OUTLINE2,
                shadow: ctrl_id::NAME_SHADOW_COLOR,
                font: ctrl_id::NAME_FONT,
                shadow_check: ctrl_id::NAME_SHADOW,
                shadow_enabled: self.config.borrow().name_style.shadow_enabled,
            };
            self.create_text_style_group(tab, tx, cy, 150, "이름 설정", name_spec)?;

            let org_spec = TextStyleGroupSpec {
                color: ctrl_id::ORG_COLOR,
                outline1: ctrl_id::ORG_OUTLINE1,
                outline2: ctrl_id::ORG_OUTLINE2,
                shadow: ctrl_id::ORG_SHADOW_COLOR,
                font: ctrl_id::ORG_FONT,
                shadow_check: ctrl_id::ORG_SHADOW,
                shadow_enabled: self.config.borrow().original_style.shadow_enabled,
            };
            self.create_text_style_group(tab, tx + 155, cy, 150, "원문 설정", org_spec)?;

            let trans_spec = TextStyleGroupSpec {
                color: ctrl_id::TRANS_COLOR,
                outline1: ctrl_id::TRANS_OUTLINE1,
                outline2: ctrl_id::TRANS_OUTLINE2,
                shadow: ctrl_id::TRANS_SHADOW_COLOR,
                font: ctrl_id::TRANS_FONT,
                shadow_check: ctrl_id::TRANS_SHADOW,
                shadow_enabled: self.config.borrow().translation_style.shadow_enabled,
            };
            self.create_text_style_group(tab, tx + 310, cy, 150, "번역문 설정", trans_spec)?;

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
        spec: TextStyleGroupSpec,
    ) -> Result<()> {
        unsafe {
            // 좌/우 마진 5/6 (1px 비대칭), 버튼 폭 (w-15)/2 = 67, 간격 5.
            // 기존 (w-20)/2=65 + 좌5/우10 비대칭을 좁힌 결과.
            let bw = (w - 15) / 2;  // 버튼 너비

            let h = self.create_group_box(x, y, w, 125, title)?;
            self.register_control(tab, h);

            let h = self.create_color_button(x + 5, y + 20, bw, 22, spec.color, "주색상")?;
            self.register_control(tab, h);
            let h = self.create_color_button(x + 5 + bw + 5, y + 20, bw, 22, spec.outline1, "외곽1")?;
            self.register_control(tab, h);
            let h = self.create_color_button(x + 5, y + 47, bw, 22, spec.outline2, "외곽2")?;
            self.register_control(tab, h);
            let h = self.create_color_button(x + 5 + bw + 5, y + 47, bw, 22, spec.shadow, "그림자색")?;
            self.register_control(tab, h);
            let h = self.create_button(x + 5, y + 74, w - 11, 22, spec.font, "폰트 선택")?;
            self.register_control(tab, h);
            let h = self.create_checkbox(x + 5, y + 100, 110, 20, spec.shadow_check, "그림자 사용", spec.shadow_enabled)?;
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
            let h = self.create_checkbox(tx + 360, ty + 25, 100, 20, ctrl_id::SEPERATE_NAME, "이름 줄바꿈", sep_name)?;
            self.register_control(tab, h);

            let repeat_mode = self.config.borrow().repeat_text_mode;
            let h = self.create_button(tx + 15, ty + 55, 90, 24, ctrl_id::REPEAT_TEXT, &repeat_mode_label(repeat_mode))?;
            self.register_control(tab, h);

            let align = self.config.borrow().text_align;
            let h = self.create_radio(tx + 120, ty + 58, 55, 20, ctrl_id::TEXTALIGN_LEFT, "왼쪽", align == TextAlign::Left)?;
            self.register_control(tab, h);
            let h = self.create_radio(tx + 185, ty + 58, 55, 20, ctrl_id::TEXTALIGN_MID, "중앙", align == TextAlign::Center)?;
            self.register_control(tab, h);
            let h = self.create_radio(tx + 250, ty + 58, 65, 20, ctrl_id::TEXTALIGN_RIGHT, "오른쪽", align == TextAlign::Right)?;
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
            let h = self.create_checkbox(tx + 195, wy + 25, 110, 20, ctrl_id::MAGNETIC_MINIMIZE, "자석 최소화", magnetic_min)?;
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

            // 소스/타겟 언어 — 항목은 엔진별로 다르므로 빈 콤보로 생성 후 apply_engine_state에서 채움
            let h = self.create_label(tx + 160, ty + 25, 40, 18, "소스:")?;
            self.register_control(tab, h);
            let h = self.create_combobox(tx + 200, ty + 23, 80, 200, ctrl_id::TRANS_SOURCE_LANG, &[], 0)?;
            self.register_control(tab, h);

            let h = self.create_label(tx + 295, ty + 25, 40, 18, "타겟:")?;
            self.register_control(tab, h);
            // 타겟 콤보 폭 80→70 으로 줄여 우측 "자동" 체크박스 공간 확보.
            let h = self.create_combobox(tx + 335, ty + 23, 70, 200, ctrl_id::TRANS_TARGET_LANG, &[], 0)?;
            self.register_control(tab, h);

            let auto_detect = self.config.borrow().translation.auto_detect;
            // 타겟 콤보 우측 끝 tx+405. 그룹박스 우측 끝 tx+460.
            // x=tx+410, w=45 → 우측 끝 tx+455 → 그룹 안쪽 5px 여유.
            let h = self.create_checkbox(tx + 410, ty + 25, 45, 18, ctrl_id::TRANS_AUTO_DETECT, "자동", auto_detect)?;
            self.register_control(tab, h);

            // EzTrans 경로
            let h = self.create_label(tx + 15, ty + 60, 80, 18, "EzTrans DLL:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::EzTrans, h);
            let dll_path = self.config.borrow().translation.eztrans_dll_path.clone();
            let h = self.create_edit(tx + 95, ty + 58, 280, 22, ctrl_id::EZTRANS_DLL_EDIT, &dll_path)?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::EzTrans, h);
            let h = self.create_button(tx + 380, ty + 58, 70, 22, ctrl_id::EZTRANS_DLL_BROWSE, "찾아보기")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::EzTrans, h);

            let h = self.create_label(tx + 15, ty + 90, 80, 18, "EzTrans Dat:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::EzTrans, h);
            let dat_path = self.config.borrow().translation.eztrans_dat_path.clone();
            let h = self.create_edit(tx + 95, ty + 88, 280, 22, ctrl_id::EZTRANS_DAT_EDIT, &dat_path)?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::EzTrans, h);
            let h = self.create_button(tx + 380, ty + 88, 70, 22, ctrl_id::EZTRANS_DAT_BROWSE, "찾아보기")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::EzTrans, h);

            // ── DeepL 그룹 ──
            let dy = ty + 140;
            let h = self.create_group_box(tx, dy, 460, 155, "DeepL")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::DeepL, h);

            // 단일 키 (deepl_keys 비어있을 때만 사용)
            let h = self.create_label(tx + 15, dy + 25, 80, 18, "API 키 (단일):")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::DeepL, h);
            let api_key = self.config.borrow().translation.deepl_api_key.clone();
            let h = self.create_edit(tx + 100, dy + 23, 345, 22, ctrl_id::DEEPL_API_KEY_EDIT, &api_key)?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::DeepL, h);

            // 안내 문구: 단일 키 vs 보조 키 사용 규칙
            let h = self.create_label(tx + 15, dy + 50, 430, 16, "* 보조 키 목록이 비어있을 때만 단일 키를 사용합니다.")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::DeepL, h);

            // 보조 키 목록 (멀티 키)
            let h = self.create_label(tx + 15, dy + 70, 100, 18, "보조 키 목록:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::DeepL, h);
            let h = self.create_label(tx + 305, dy + 70, 50, 18, "전략:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::DeepL, h);
            let strategy_items = vec!["failover", "round-robin"];
            let strategy_sel: usize = match self.config.borrow().translation.deepl_strategy.to_lowercase().as_str() {
                "round-robin" | "roundrobin" | "rr" => 1,
                _ => 0,
            };
            let h = self.create_combobox(tx + 345, dy + 68, 100, 120, ctrl_id::DEEPL_STRATEGY_COMBO, &strategy_items, strategy_sel)?;
            // 안전망: 일부 환경에서 생성 직후 CB_SETCURSEL이 무시되는 경우가 있어 한 번 더 적용
            let _ = SendMessageW(h, CB_SETCURSEL, Some(WPARAM(strategy_sel)), Some(LPARAM(0)));
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::DeepL, h);

            let listbox = self.create_listbox(tx + 15, dy + 92, 285, 55, ctrl_id::DEEPL_KEYS_LIST)?;
            self.register_control(tab, listbox);
            self.register_engine_control(EngineGroup::DeepL, listbox);
            // 초기 항목 채우기
            {
                let cfg = self.config.borrow();
                for k in &cfg.translation.deepl_keys {
                    let kw = to_wide(k);
                    let _ = SendMessageW(listbox, LB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(kw.as_ptr() as isize)));
                }
            }

            let h = self.create_edit(tx + 305, dy + 92, 140, 22, ctrl_id::DEEPL_KEY_ADD_EDIT, "")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::DeepL, h);
            let h = self.create_button(tx + 305, dy + 120, 65, 22, ctrl_id::DEEPL_KEY_ADD_BTN, "추가")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::DeepL, h);
            let h = self.create_button(tx + 380, dy + 120, 65, 22, ctrl_id::DEEPL_KEY_REMOVE_BTN, "삭제")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::DeepL, h);

            // ── Papago 그룹 ──
            let py = ty + 305;
            let h = self.create_group_box(tx, py, 460, 70, "Papago")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Papago, h);

            let h = self.create_label(tx + 15, py + 22, 80, 18, "Client ID:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Papago, h);
            let papago_id = self.config.borrow().translation.papago_client_id.clone();
            let h = self.create_edit(tx + 100, py + 20, 345, 22, ctrl_id::PAPAGO_ID_EDIT, &papago_id)?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Papago, h);

            let h = self.create_label(tx + 15, py + 47, 80, 18, "Secret:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Papago, h);
            let papago_secret = self.config.borrow().translation.papago_client_secret.clone();
            let h = self.create_edit(tx + 100, py + 45, 345, 22, ctrl_id::PAPAGO_SECRET_EDIT, &papago_secret)?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Papago, h);

            // ── LLM 번역 그룹 ──
            let ly = ty + 385;
            let h = self.create_group_box(tx, ly, 460, 345, "LLM 번역")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);

            // 제공자 + 모델
            let h = self.create_label(tx + 15, ly + 25, 50, 18, "제공자:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);
            // 표시 라벨은 LlmProvider::display_name 와 동기화 (OpenAI API / Claude API / AI Studio / Grok / OpenRouter)
            let provider_items: Vec<&'static str> = crate::translation::LlmProvider::ALL
                .iter()
                .map(|p| p.display_name())
                .collect();
            let provider_sel = self.config.borrow().translation.llm.get_provider() as u8 as usize;
            let h = self.create_combobox(tx + 65, ly + 23, 100, 150, ctrl_id::LLM_PROVIDER, &provider_items, provider_sel)?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);

            let h = self.create_label(tx + 180, ly + 25, 40, 18, "모델:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);
            let model = self.config.borrow().translation.llm.model.clone();
            let h = self.create_edit(tx + 220, ly + 23, 230, 22, ctrl_id::LLM_MODEL_EDIT, &model)?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);

            // API 키
            let h = self.create_label(tx + 15, ly + 55, 50, 18, "API 키:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);
            let api_key = self.config.borrow().translation.llm.api_key.clone();
            let h = self.create_edit(tx + 65, ly + 53, 385, 22, ctrl_id::LLM_API_KEY_EDIT, &api_key)?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);

            // Base URL
            let h = self.create_label(tx + 15, ly + 85, 60, 18, "Base URL:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);
            let base_url = self.config.borrow().translation.llm.base_url.clone();
            let h = self.create_edit(tx + 80, ly + 83, 370, 22, ctrl_id::LLM_BASE_URL_EDIT, &base_url)?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);

            // 시스템 프롬프트 (멀티라인) — 60 → 110px 로 확장
            let h = self.create_label(tx + 15, ly + 115, 100, 18, "시스템 프롬프트:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);
            let system_prompt = self.config.borrow().translation.llm.system_prompt.clone();
            let h = self.create_multiline_edit(tx + 15, ly + 135, 435, 110, ctrl_id::LLM_SYSTEM_PROMPT_EDIT, &system_prompt)?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);

            // Temperature 트랙바 (0.00 ~ 2.00, 100 단위 = 0~200)
            let temperature_value = self.config.borrow().translation.llm.temperature;
            let h = self.create_label(tx + 15, ly + 255, 90, 18, "Temperature:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);
            let temp_tb = self.create_trackbar(tx + 105, ly + 253, 180, 22, ctrl_id::LLM_TEMPERATURE_TRACKBAR, 0, 200)?;
            self.register_control(tab, temp_tb);
            self.register_engine_control(EngineGroup::Llm, temp_tb);
            let temp_pos = (temperature_value * 100.0).clamp(0.0, 200.0) as isize;
            let _ = SendMessageW(temp_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(temp_pos)));
            let h = self.create_label_with_id(tx + 290, ly + 257, 50, 18, ctrl_id::LLM_TEMPERATURE_LABEL, &format!("{:.2}", temperature_value))?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);

            // Max Tokens — 라벨 폭 30→42, edit 시작 위치 우측으로
            let h = self.create_label(tx + 345, ly + 257, 42, 18, "Max:")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);
            let max_tokens = format!("{}", self.config.borrow().translation.llm.max_tokens);
            let h = self.create_edit_numeric(tx + 390, ly + 255, 60, 22, ctrl_id::LLM_MAX_TOKENS_EDIT, &max_tokens)?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);

            // Debounce + Glossary 편집
            let h = self.create_label(tx + 15, ly + 287, 90, 18, "디바운스(ms):")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);
            let debounce = format!("{}", self.config.borrow().translation.llm.debounce_ms);
            let h = self.create_edit_numeric(tx + 105, ly + 285, 70, 22, ctrl_id::LLM_DEBOUNCE_EDIT, &debounce)?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);

            let glossary_count = self.config.borrow().translation.llm.glossary.len();
            let h = self.create_label_with_id(tx + 195, ly + 287, 120, 18, ctrl_id::LLM_GLOSSARY_COUNT_LABEL, &format!("사전 항목: {}", glossary_count))?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);
            let h = self.create_button(tx + 325, ly + 285, 125, 22, ctrl_id::LLM_GLOSSARY_EDIT_BTN, "사전 편집...")?;
            self.register_control(tab, h);
            self.register_engine_control(EngineGroup::Llm, h);

            Ok(())
        }
    }

    /// 커스텀 메시지 핸들러
    fn handle_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_CTLCOLORSTATIC => {
                // 탭 컨트롤 본문은 비주얼 스타일이 흰색(COLOR_WINDOW)으로 그리는데
                // 그 위에 올라간 STATIC/체크박스/라디오 자식은 다이얼로그의 회색
                // brush(COLOR_BTNFACE)로 칠해져 흰 본문 위 회색 사각형으로 도드라진다.
                // Settings 의 거의 모든 라벨이 탭 위에 있으므로 일괄 흰 brush 반환.
                // SAFETY: wparam 은 OS 가 넘긴 유효 HDC. SetBkMode/GetSysColorBrush
                // 는 표준 GDI 호출.
                unsafe {
                    let hdc = HDC(wparam.0 as *mut _);
                    let _ = SetBkMode(hdc, TRANSPARENT);
                    let brush = GetSysColorBrush(COLOR_WINDOW);
                    Some(LRESULT(brush.0 as isize))
                }
            }
            WM_DRAWITEM => {
                // SAFETY: lparam points to a valid DRAWITEMSTRUCT from the system.
                unsafe {
                    let dis = &*(lparam.0 as *const DRAWITEMSTRUCT);
                    if dis.CtlType == ODT_BUTTON {
                        self.draw_color_button(dis);
                        return Some(LRESULT(1));
                    }
                }
                None
            }
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
                    if nmhdr.code == TCN_SELCHANGE
                        && let Ok(tab_hwnd) = GetDlgItem(Some(self.hwnd), ctrl_id::TAB_CONTROL as i32)
                    {
                        let sel = SendMessageW(
                            tab_hwnd,
                            TCM_GETCURSEL,
                            Some(WPARAM(0)),
                            Some(LPARAM(0)),
                        ).0 as usize;
                        self.switch_tab(sel);
                    }
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}
