//! 설정 대화상자
//!
//! 모든 설정을 관리하는 메인 설정 대화상자.
//! Win32 CreateWindowEx를 사용하여 컨트롤을 프로그래매틱하게 생성.

use std::cell::RefCell;
use std::rc::Rc;

use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::Controls::*,
        UI::WindowsAndMessaging::*,
    },
};

use crate::config::{ColorType, Config, TextAlign, TextType};
use super::color::ColorDialog;
use super::font::{FontDialog, FontDialogConfig, FontStyle};

// TrackBar 메시지 상수 (windows crate 0.62에서 누락)
const TBM_GETPOS_VAL: u32 = 1024;

// ComboBox 메시지 상수
const CB_ADDSTRING: u32 = 0x0143;
const CB_SETCURSEL: u32 = 0x014E;
const CB_GETCURSEL: u32 = 0x0147;
const CBN_SELCHANGE: u32 = 1;

// 설정 대화상자 컨트롤 ID
mod ctrl_id {
    // 배경 설정
    pub const BACKGROUND_TRACKBAR: u16 = 1001;
    pub const BACKGROUND_COLOR: u16 = 1002;
    pub const BACKGROUND_SWITCH: u16 = 1003;

    // 텍스트 크기
    pub const TEXTSIZE_TRACKBAR: u16 = 1010;
    pub const TEXTSIZE_MINUS: u16 = 1011;
    pub const TEXTSIZE_PLUS: u16 = 1012;
    pub const TEXTSIZE_TEXT: u16 = 1013;

    // 외곽선 크기
    pub const OUTLINE1_TRACKBAR: u16 = 1020;
    pub const OUTLINE1_MINUS: u16 = 1021;
    pub const OUTLINE1_PLUS: u16 = 1022;
    pub const OUTLINE1_TEXT: u16 = 1023;

    pub const OUTLINE2_TRACKBAR: u16 = 1030;
    pub const OUTLINE2_MINUS: u16 = 1031;
    pub const OUTLINE2_PLUS: u16 = 1032;
    pub const OUTLINE2_TEXT: u16 = 1033;

    // 그림자 오프셋
    pub const SHADOW_X_TRACKBAR: u16 = 1040;
    pub const SHADOW_X_TEXT: u16 = 1041;
    pub const SHADOW_Y_TRACKBAR: u16 = 1042;
    pub const SHADOW_Y_TEXT: u16 = 1043;

    // 텍스트 여백
    pub const MARGIN_X_TRACKBAR: u16 = 1050;
    pub const MARGIN_X_TEXT: u16 = 1051;
    pub const MARGIN_Y_TRACKBAR: u16 = 1052;
    pub const MARGIN_Y_TEXT: u16 = 1053;
    pub const MARGIN_NAME_TRACKBAR: u16 = 1054;
    pub const MARGIN_NAME_TEXT: u16 = 1055;

    // NAME 설정
    pub const NAME_COLOR: u16 = 1100;
    pub const NAME_OUTLINE1: u16 = 1101;
    pub const NAME_OUTLINE2: u16 = 1102;
    pub const NAME_SHADOW_COLOR: u16 = 1103;
    pub const NAME_FONT: u16 = 1104;
    pub const NAME_SHADOW: u16 = 1105;

    // ORG 설정
    pub const ORG_COLOR: u16 = 1110;
    pub const ORG_OUTLINE1: u16 = 1111;
    pub const ORG_OUTLINE2: u16 = 1112;
    pub const ORG_SHADOW_COLOR: u16 = 1113;
    pub const ORG_FONT: u16 = 1114;
    pub const ORG_SHADOW: u16 = 1115;

    // TRANS 설정
    pub const TRANS_COLOR: u16 = 1120;
    pub const TRANS_OUTLINE1: u16 = 1121;
    pub const TRANS_OUTLINE2: u16 = 1122;
    pub const TRANS_SHADOW_COLOR: u16 = 1123;
    pub const TRANS_FONT: u16 = 1124;
    pub const TRANS_SHADOW: u16 = 1125;

    // 테두리 설정
    pub const BORDER_MODE: u16 = 1130;
    pub const BORDER_COLOR: u16 = 1131;
    pub const BORDER_SIZE_TRACKBAR: u16 = 1132;
    pub const BORDER_SIZE_TEXT: u16 = 1133;

    // 표시 옵션
    pub const PRINT_ORGTEXT: u16 = 1200;
    pub const PRINT_TRANSTEXT: u16 = 1201;
    pub const PRINT_ORGNAME: u16 = 1202;
    pub const SEPERATE_NAME: u16 = 1203;
    pub const REPEAT_TEXT: u16 = 1204;

    // 윈도우 옵션
    pub const TOPMOST: u16 = 1210;
    pub const USE_MAGNETIC: u16 = 1211;
    pub const MAGNETIC_MINIMIZE: u16 = 1212;
    pub const HIDEWIN: u16 = 1213;
    pub const CLIPBOARD_WATCH: u16 = 1214;
    pub const WNDCLICK_THROUGH: u16 = 1215;

    // 텍스트 정렬
    pub const TEXTALIGN_LEFT: u16 = 1220;
    pub const TEXTALIGN_MID: u16 = 1221;
    pub const TEXTALIGN_RIGHT: u16 = 1222;

    // 스크린샷 설정
    pub const SCREENSHOT_PATH_EDIT: u16 = 1250;
    pub const SCREENSHOT_PATH_BROWSE: u16 = 1251;
    pub const SCREENSHOT_FORMAT: u16 = 1252;
    pub const SCREENSHOT_COMPRESSION: u16 = 1253;
    pub const SCREENSHOT_QUALITY_TRACKBAR: u16 = 1254;
    pub const SCREENSHOT_QUALITY_TEXT: u16 = 1255;

    // 닫기 버튼
    pub const CLOSE: u16 = 1300;
}

const SETTINGS_CLASS_NAME: PCWSTR = w!("AnemoneSettingsClass");
const SETTINGS_WIDTH: i32 = 500;
const SETTINGS_HEIGHT: i32 = 800;

/// 설정 변경 콜백 타입
pub type SettingsChangeCallback = Box<dyn Fn(&Config)>;

/// 설정 대화상자
pub struct SettingsDialog {
    hwnd: HWND,
    config: Rc<RefCell<Config>>,
    main_hwnd: HWND,
    on_change: Option<SettingsChangeCallback>,
}

thread_local! {
    static SETTINGS_INSTANCE: RefCell<Option<Rc<RefCell<SettingsDialog>>>> = const { RefCell::new(None) };
}

impl SettingsDialog {
    /// 설정 대화상자 생성 및 표시
    pub fn show(
        main_hwnd: HWND,
        config: Rc<RefCell<Config>>,
        on_change: Option<SettingsChangeCallback>,
    ) -> Result<HWND> {
        unsafe { Self::show_impl(main_hwnd, config, on_change) }
    }

    unsafe fn show_impl(
        main_hwnd: HWND,
        config: Rc<RefCell<Config>>,
        on_change: Option<SettingsChangeCallback>,
    ) -> Result<HWND> { unsafe {
        let instance = GetModuleHandleW(None)?;

        // 윈도우 클래스 등록
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(Self::wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance.into(),
            hIcon: LoadIconW(None, IDI_APPLICATION)?,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as *mut _),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: SETTINGS_CLASS_NAME,
            hIconSm: HICON::default(),
        };

        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            let err = GetLastError();
            if err != ERROR_CLASS_ALREADY_EXISTS {
                return Err(Error::from_hresult(HRESULT::from_win32(err.0)));
            }
        }

        // 화면 중앙에 위치
        let cx = GetSystemMetrics(SM_CXSCREEN);
        let cy = GetSystemMetrics(SM_CYSCREEN);
        let x = (cx - SETTINGS_WIDTH) / 2;
        let y = (cy - SETTINGS_HEIGHT) / 2;

        // 윈도우 생성
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            SETTINGS_CLASS_NAME,
            w!("아네모네 설정"),
            WS_POPUP | WS_CAPTION | WS_SYSMENU,
            x,
            y,
            SETTINGS_WIDTH,
            SETTINGS_HEIGHT,
            Some(main_hwnd),
            None,
            Some(instance.into()),
            None,
        )?;

        // 인스턴스 생성
        let dialog = Rc::new(RefCell::new(SettingsDialog {
            hwnd,
            config,
            main_hwnd,
            on_change,
        }));

        // 전역 인스턴스 설정
        SETTINGS_INSTANCE.with(|cell| {
            *cell.borrow_mut() = Some(dialog.clone());
        });

        // 컨트롤 생성
        dialog.borrow_mut().create_controls()?;

        // 윈도우 표시
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);

        Ok(hwnd)
    }}

    /// 컨트롤 생성
    fn create_controls(&mut self) -> Result<()> {
        unsafe {
            let _hinst = GetModuleHandleW(None)?;
            let _hfont = GetStockObject(DEFAULT_GUI_FONT);

            // ====== 배경 설정 그룹 ======
            self.create_group_box(10, 10, 230, 80, "배경 설정")?;

            // 배경 투명도 트랙바
            self.create_label(20, 30, 60, 18, "투명도:")?;
            let trackbar = self.create_trackbar(
                80,
                28,
                150,
                22,
                ctrl_id::BACKGROUND_TRACKBAR,
                0,
                255,
            )?;
            let alpha = (self.config.borrow().background_color >> 24) & 0xFF;
            let _ = SendMessageW(trackbar, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(alpha as isize)));

            // 배경 색상 버튼
            self.create_color_button(20, 55, 70, 25, ctrl_id::BACKGROUND_COLOR, "배경색")?;

            // 배경 표시 체크박스
            let bg_visible = self.config.borrow().background_visible;
            self.create_checkbox(100, 57, 80, 20, ctrl_id::BACKGROUND_SWITCH, "표시", bg_visible)?;

            // ====== 텍스트 크기 그룹 ======
            self.create_group_box(250, 10, 230, 80, "텍스트 크기")?;

            // 텍스트 크기 트랙바
            let size_trackbar = self.create_trackbar(
                260,
                30,
                180,
                22,
                ctrl_id::TEXTSIZE_TRACKBAR,
                6,
                100,
            )?;
            let text_size = self.config.borrow().translation_style.size;
            let _ = SendMessageW(size_trackbar, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(text_size as isize)));

            // +/- 버튼
            self.create_button(260, 55, 30, 25, ctrl_id::TEXTSIZE_MINUS, "-")?;
            self.create_button(295, 55, 30, 25, ctrl_id::TEXTSIZE_PLUS, "+")?;

            // 크기 텍스트
            self.create_label(330, 58, 100, 18, &format!("크기: {}", text_size))?;

            // ====== 외곽선 설정 그룹 ======
            self.create_group_box(10, 95, 470, 80, "외곽선 설정")?;

            // 외곽선1
            self.create_label(20, 115, 60, 18, "외곽선1:")?;
            let outline1_tb = self.create_trackbar(
                80,
                113,
                120,
                22,
                ctrl_id::OUTLINE1_TRACKBAR,
                0,
                20,
            )?;
            let outline1_size = self.config.borrow().translation_style.outline1_size;
            let _ = SendMessageW(outline1_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(outline1_size as isize)));

            self.create_button(205, 113, 25, 22, ctrl_id::OUTLINE1_MINUS, "-")?;
            self.create_button(232, 113, 25, 22, ctrl_id::OUTLINE1_PLUS, "+")?;

            // 외곽선2
            self.create_label(270, 115, 60, 18, "외곽선2:")?;
            let outline2_tb = self.create_trackbar(
                330,
                113,
                120,
                22,
                ctrl_id::OUTLINE2_TRACKBAR,
                0,
                20,
            )?;
            let outline2_size = self.config.borrow().translation_style.outline2_size;
            let _ = SendMessageW(outline2_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(outline2_size as isize)));

            self.create_button(455, 113, 25, 22, ctrl_id::OUTLINE2_MINUS, "-")?;
            // OUTLINE2_PLUS 버튼은 공간 부족으로 두 번째 줄에 배치
            self.create_button(455, 140, 25, 22, ctrl_id::OUTLINE2_PLUS, "+")?;

            // 그림자 오프셋
            self.create_label(20, 145, 80, 18, "그림자 X:")?;
            let shadow_x_tb = self.create_trackbar(
                100,
                143,
                100,
                22,
                ctrl_id::SHADOW_X_TRACKBAR,
                0,
                20,
            )?;
            let shadow_x = self.config.borrow().shadow_offset_x;
            let _ = SendMessageW(shadow_x_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(shadow_x as isize)));

            self.create_label(220, 145, 80, 18, "그림자 Y:")?;
            let shadow_y_tb = self.create_trackbar(
                300,
                143,
                100,
                22,
                ctrl_id::SHADOW_Y_TRACKBAR,
                0,
                20,
            )?;
            let shadow_y = self.config.borrow().shadow_offset_y;
            let _ = SendMessageW(shadow_y_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(shadow_y as isize)));

            // ====== 이름(NAME) 설정 그룹 ======
            self.create_group_box(10, 180, 155, 130, "이름 설정")?;
            self.create_color_button(20, 200, 65, 22, ctrl_id::NAME_COLOR, "주색상")?;
            self.create_color_button(90, 200, 65, 22, ctrl_id::NAME_OUTLINE1, "외곽1")?;
            self.create_color_button(20, 227, 65, 22, ctrl_id::NAME_OUTLINE2, "외곽2")?;
            self.create_color_button(90, 227, 65, 22, ctrl_id::NAME_SHADOW_COLOR, "그림자")?;
            self.create_button(20, 254, 135, 22, ctrl_id::NAME_FONT, "폰트 선택")?;
            let name_shadow = self.config.borrow().name_style.shadow_enabled;
            self.create_checkbox(20, 280, 100, 20, ctrl_id::NAME_SHADOW, "그림자", name_shadow)?;

            // ====== 원문(ORG) 설정 그룹 ======
            self.create_group_box(170, 180, 155, 130, "원문 설정")?;
            self.create_color_button(180, 200, 65, 22, ctrl_id::ORG_COLOR, "주색상")?;
            self.create_color_button(250, 200, 65, 22, ctrl_id::ORG_OUTLINE1, "외곽1")?;
            self.create_color_button(180, 227, 65, 22, ctrl_id::ORG_OUTLINE2, "외곽2")?;
            self.create_color_button(250, 227, 65, 22, ctrl_id::ORG_SHADOW_COLOR, "그림자")?;
            self.create_button(180, 254, 135, 22, ctrl_id::ORG_FONT, "폰트 선택")?;
            let org_shadow = self.config.borrow().original_style.shadow_enabled;
            self.create_checkbox(180, 280, 100, 20, ctrl_id::ORG_SHADOW, "그림자", org_shadow)?;

            // ====== 번역(TRANS) 설정 그룹 ======
            self.create_group_box(330, 180, 150, 130, "번역 설정")?;
            self.create_color_button(340, 200, 65, 22, ctrl_id::TRANS_COLOR, "주색상")?;
            self.create_color_button(410, 200, 60, 22, ctrl_id::TRANS_OUTLINE1, "외곽1")?;
            self.create_color_button(340, 227, 65, 22, ctrl_id::TRANS_OUTLINE2, "외곽2")?;
            self.create_color_button(410, 227, 60, 22, ctrl_id::TRANS_SHADOW_COLOR, "그림자")?;
            self.create_button(340, 254, 130, 22, ctrl_id::TRANS_FONT, "폰트 선택")?;
            let trans_shadow = self.config.borrow().translation_style.shadow_enabled;
            self.create_checkbox(340, 280, 100, 20, ctrl_id::TRANS_SHADOW, "그림자", trans_shadow)?;

            // ====== 텍스트 여백 그룹 ======
            self.create_group_box(10, 315, 470, 60, "텍스트 여백")?;
            self.create_label(20, 335, 50, 18, "좌우:")?;
            let margin_x_tb = self.create_trackbar(
                70,
                333,
                100,
                22,
                ctrl_id::MARGIN_X_TRACKBAR,
                0,
                300,
            )?;
            let margin_x = self.config.borrow().text_margin_x;
            let _ = SendMessageW(margin_x_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(margin_x as isize)));

            self.create_label(180, 335, 50, 18, "상하:")?;
            let margin_y_tb = self.create_trackbar(
                230,
                333,
                100,
                22,
                ctrl_id::MARGIN_Y_TRACKBAR,
                0,
                300,
            )?;
            let margin_y = self.config.borrow().text_margin_y;
            let _ = SendMessageW(margin_y_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(margin_y as isize)));

            self.create_label(340, 335, 50, 18, "이름:")?;
            let margin_name_tb = self.create_trackbar(
                390,
                333,
                80,
                22,
                ctrl_id::MARGIN_NAME_TRACKBAR,
                0,
                300,
            )?;
            let margin_name = self.config.borrow().name_margin;
            let _ = SendMessageW(margin_name_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(margin_name as isize)));

            // ====== 표시 옵션 그룹 ======
            self.create_group_box(10, 380, 230, 100, "표시 옵션")?;
            let show_org = self.config.borrow().show_original;
            self.create_checkbox(20, 400, 100, 20, ctrl_id::PRINT_ORGTEXT, "원문 표시", show_org)?;
            let show_trans = self.config.borrow().show_translation;
            self.create_checkbox(120, 400, 100, 20, ctrl_id::PRINT_TRANSTEXT, "번역 표시", show_trans)?;
            let show_name = self.config.borrow().show_name;
            self.create_checkbox(20, 420, 100, 20, ctrl_id::PRINT_ORGNAME, "이름 표시", show_name)?;
            let sep_name = self.config.borrow().separate_name;
            self.create_checkbox(120, 420, 100, 20, ctrl_id::SEPERATE_NAME, "이름 분리", sep_name)?;

            // 텍스트 반복 버튼
            let repeat_mode = self.config.borrow().repeat_text_mode;
            self.create_button(20, 445, 60, 22, ctrl_id::REPEAT_TEXT, &format!("반복:{}", repeat_mode))?;

            // 텍스트 정렬
            let align = self.config.borrow().text_align;
            self.create_radio(90, 448, 50, 20, ctrl_id::TEXTALIGN_LEFT, "왼쪽", align == TextAlign::Left)?;
            self.create_radio(145, 448, 50, 20, ctrl_id::TEXTALIGN_MID, "중앙", align == TextAlign::Center)?;
            self.create_radio(200, 448, 40, 20, ctrl_id::TEXTALIGN_RIGHT, "오른쪽", align == TextAlign::Right)?;

            // ====== 윈도우 옵션 그룹 ======
            self.create_group_box(250, 380, 230, 100, "윈도우 옵션")?;
            let topmost = self.config.borrow().window_topmost;
            self.create_checkbox(260, 400, 100, 20, ctrl_id::TOPMOST, "항상 위", topmost)?;
            let magnetic = self.config.borrow().magnetic_mode;
            self.create_checkbox(360, 400, 80, 20, ctrl_id::USE_MAGNETIC, "자석", magnetic)?;
            let magnetic_min = self.config.borrow().magnetic_minimize;
            self.create_checkbox(435, 400, 45, 20, ctrl_id::MAGNETIC_MINIMIZE, "최소", magnetic_min)?;
            let hide_win = self.config.borrow().temp_window_hide;
            self.create_checkbox(260, 420, 60, 20, ctrl_id::HIDEWIN, "숨기기", hide_win)?;
            let clip_watch = self.config.borrow().clipboard_watch;
            self.create_checkbox(320, 420, 70, 20, ctrl_id::CLIPBOARD_WATCH, "클립보드", clip_watch)?;
            let click_through = self.config.borrow().click_through;
            self.create_checkbox(395, 420, 80, 20, ctrl_id::WNDCLICK_THROUGH, "클릭 통과", click_through)?;

            // ====== 테두리 설정 그룹 ======
            self.create_group_box(10, 485, 470, 60, "테두리 설정")?;
            let border_visible = self.config.borrow().border_visible;
            self.create_checkbox(20, 505, 80, 20, ctrl_id::BORDER_MODE, "표시", border_visible)?;
            self.create_color_button(100, 503, 80, 22, ctrl_id::BORDER_COLOR, "테두리색")?;

            self.create_label(190, 507, 50, 18, "두께:")?;
            let border_tb = self.create_trackbar(
                240,
                505,
                150,
                22,
                ctrl_id::BORDER_SIZE_TRACKBAR,
                0,
                10,
            )?;
            let border_size = self.config.borrow().border_width;
            let _ = SendMessageW(border_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(border_size as isize)));

            // ====== 스크린샷 설정 그룹 ======
            self.create_group_box(10, 550, 470, 110, "스크린샷 설정")?;

            // 저장 경로
            self.create_label(20, 572, 50, 18, "경로:")?;
            let screenshot_path = self.config.borrow().screenshot.path.clone();
            self.create_edit(70, 570, 330, 22, ctrl_id::SCREENSHOT_PATH_EDIT, &screenshot_path)?;
            self.create_button(405, 570, 65, 22, ctrl_id::SCREENSHOT_PATH_BROWSE, "찾아보기")?;

            // 포맷 선택
            self.create_label(20, 600, 40, 18, "포맷:")?;
            let format_items = vec!["PNG", "JPEG", "WebP"];
            let format_sel = self.config.borrow().screenshot.format as usize;
            self.create_combobox(60, 598, 70, 100, ctrl_id::SCREENSHOT_FORMAT, &format_items, format_sel)?;

            // 압축 레벨
            self.create_label(140, 600, 40, 18, "압축:")?;
            let compress_items = vec!["빠름", "표준", "최대"];
            let compress_sel = self.config.borrow().screenshot.compression as usize;
            self.create_combobox(180, 598, 60, 100, ctrl_id::SCREENSHOT_COMPRESSION, &compress_items, compress_sel)?;

            // JPEG 품질
            self.create_label(250, 600, 60, 18, "JPEG품질:")?;
            let quality_tb = self.create_trackbar(
                310,
                598,
                100,
                22,
                ctrl_id::SCREENSHOT_QUALITY_TRACKBAR,
                1,
                100,
            )?;
            let jpeg_quality = self.config.borrow().screenshot.jpeg_quality;
            let _ = SendMessageW(quality_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(jpeg_quality as isize)));
            self.create_label_with_id(415, 600, 40, 18, ctrl_id::SCREENSHOT_QUALITY_TEXT, &format!("{}", jpeg_quality))?;

            // ====== 닫기 버튼 ======
            self.create_button(380, 720, 100, 30, ctrl_id::CLOSE, "닫기")?;

            Ok(())
        }
    }

    // 헬퍼 함수들
    unsafe fn create_group_box(&self, x: i32, y: i32, w: i32, h: i32, text: &str) -> Result<HWND> { unsafe {
        let hinst = GetModuleHandleW(None)?;
        let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(BS_GROUPBOX as u32 | WS_CHILD.0 | WS_VISIBLE.0),
            x, y, w, h,
            Some(self.hwnd),
            None,
            Some(hinst.into()),
            None,
        )?;

        let hfont = GetStockObject(DEFAULT_GUI_FONT);
        let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));

        Ok(hwnd)
    }}

    unsafe fn create_label(&self, x: i32, y: i32, w: i32, h: i32, text: &str) -> Result<HWND> { unsafe {
        self.create_label_with_id(x, y, w, h, 0, text)
    }}

    unsafe fn create_label_with_id(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str) -> Result<HWND> { unsafe {
        let hinst = GetModuleHandleW(None)?;
        let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            x, y, w, h,
            Some(self.hwnd),
            Some(HMENU(id as isize as *mut _)),
            Some(hinst.into()),
            None,
        )?;

        let hfont = GetStockObject(DEFAULT_GUI_FONT);
        let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));

        Ok(hwnd)
    }}

    unsafe fn create_button(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str) -> Result<HWND> { unsafe {
        let hinst = GetModuleHandleW(None)?;
        let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(BS_PUSHBUTTON as u32 | WS_CHILD.0 | WS_VISIBLE.0),
            x, y, w, h,
            Some(self.hwnd),
            Some(HMENU(id as isize as *mut _)),
            Some(hinst.into()),
            None,
        )?;

        let hfont = GetStockObject(DEFAULT_GUI_FONT);
        let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));

        Ok(hwnd)
    }}

    unsafe fn create_color_button(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str) -> Result<HWND> { unsafe {
        let hinst = GetModuleHandleW(None)?;
        let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

        // BS_OWNERDRAW로 생성하여 색상 표시
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(BS_PUSHBUTTON as u32 | WS_CHILD.0 | WS_VISIBLE.0),
            x, y, w, h,
            Some(self.hwnd),
            Some(HMENU(id as isize as *mut _)),
            Some(hinst.into()),
            None,
        )?;

        let hfont = GetStockObject(DEFAULT_GUI_FONT);
        let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));

        Ok(hwnd)
    }}

    unsafe fn create_checkbox(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str, checked: bool) -> Result<HWND> { unsafe {
        let hinst = GetModuleHandleW(None)?;
        let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(BS_AUTOCHECKBOX as u32 | WS_CHILD.0 | WS_VISIBLE.0),
            x, y, w, h,
            Some(self.hwnd),
            Some(HMENU(id as isize as *mut _)),
            Some(hinst.into()),
            None,
        )?;

        let hfont = GetStockObject(DEFAULT_GUI_FONT);
        let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));

        if checked {
            let _ = SendMessageW(hwnd, BM_SETCHECK, Some(WPARAM(BST_CHECKED.0 as usize)), Some(LPARAM(0)));
        }

        Ok(hwnd)
    }}

    unsafe fn create_radio(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str, checked: bool) -> Result<HWND> { unsafe {
        let hinst = GetModuleHandleW(None)?;
        let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(BS_AUTORADIOBUTTON as u32 | WS_CHILD.0 | WS_VISIBLE.0),
            x, y, w, h,
            Some(self.hwnd),
            Some(HMENU(id as isize as *mut _)),
            Some(hinst.into()),
            None,
        )?;

        let hfont = GetStockObject(DEFAULT_GUI_FONT);
        let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));

        if checked {
            let _ = SendMessageW(hwnd, BM_SETCHECK, Some(WPARAM(BST_CHECKED.0 as usize)), Some(LPARAM(0)));
        }

        Ok(hwnd)
    }}

    unsafe fn create_trackbar(&self, x: i32, y: i32, w: i32, h: i32, id: u16, min: i32, max: i32) -> Result<HWND> { unsafe {
        let hinst = GetModuleHandleW(None)?;

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("msctls_trackbar32"),
            w!(""),
            WINDOW_STYLE(TBS_HORZ as u32 | TBS_NOTICKS as u32 | WS_CHILD.0 | WS_VISIBLE.0),
            x, y, w, h,
            Some(self.hwnd),
            Some(HMENU(id as isize as *mut _)),
            Some(hinst.into()),
            None,
        )?;

        // 범위 설정
        let _ = SendMessageW(hwnd, TBM_SETRANGE, Some(WPARAM(1)), Some(LPARAM(((max << 16) | min) as isize)));

        Ok(hwnd)
    }}

    unsafe fn create_edit(&self, x: i32, y: i32, w: i32, h: i32, id: u16, text: &str) -> Result<HWND> { unsafe {
        let hinst = GetModuleHandleW(None)?;
        let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

        let hwnd = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            PCWSTR(text_wide.as_ptr()),
            WINDOW_STYLE(ES_AUTOHSCROLL as u32 | WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0),
            x, y, w, h,
            Some(self.hwnd),
            Some(HMENU(id as isize as *mut _)),
            Some(hinst.into()),
            None,
        )?;

        let hfont = GetStockObject(DEFAULT_GUI_FONT);
        let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));

        Ok(hwnd)
    }}

    unsafe fn create_combobox(&self, x: i32, y: i32, w: i32, h: i32, id: u16, items: &[&str], selected: usize) -> Result<HWND> { unsafe {
        let hinst = GetModuleHandleW(None)?;

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("COMBOBOX"),
            w!(""),
            WINDOW_STYLE(CBS_DROPDOWNLIST as u32 | WS_CHILD.0 | WS_VISIBLE.0 | WS_VSCROLL.0),
            x, y, w, h,
            Some(self.hwnd),
            Some(HMENU(id as isize as *mut _)),
            Some(hinst.into()),
            None,
        )?;

        let hfont = GetStockObject(DEFAULT_GUI_FONT);
        let _ = SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(hfont.0 as usize)), Some(LPARAM(0)));

        // 아이템 추가
        for item in items {
            let item_wide: Vec<u16> = item.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = SendMessageW(hwnd, CB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(item_wide.as_ptr() as isize)));
        }

        // 선택 설정
        let _ = SendMessageW(hwnd, CB_SETCURSEL, Some(WPARAM(selected)), Some(LPARAM(0)));

        Ok(hwnd)
    }}

    /// 명령 처리
    fn handle_command(&mut self, cmd: u16) {
        use ctrl_id::*;

        match cmd {
            CLOSE => {
                unsafe {
                    let _ = DestroyWindow(self.hwnd);
                }
            }

            // 배경 색상
            BACKGROUND_COLOR => {
                let initial = self.config.borrow().background_color;
                if let Some(result) = ColorDialog::show_simple(self.hwnd, initial) {
                    self.config.borrow_mut().background_color = result.argb;
                    self.notify_change();
                }
            }

            // 배경 표시 토글
            BACKGROUND_SWITCH => {
                self.config.borrow_mut().toggle_background_visible();
                self.notify_change();
            }

            // NAME 색상들
            NAME_COLOR => self.handle_color_button(TextType::Name, ColorType::Primary),
            NAME_OUTLINE1 => self.handle_color_button(TextType::Name, ColorType::Outline1),
            NAME_OUTLINE2 => self.handle_color_button(TextType::Name, ColorType::Outline2),
            NAME_SHADOW_COLOR => self.handle_color_button(TextType::Name, ColorType::Shadow),
            NAME_FONT => self.handle_font_button(TextType::Name),
            NAME_SHADOW => {
                self.config.borrow_mut().toggle_shadow(TextType::Name);
                self.notify_change();
            }

            // ORG 색상들
            ORG_COLOR => self.handle_color_button(TextType::Original, ColorType::Primary),
            ORG_OUTLINE1 => self.handle_color_button(TextType::Original, ColorType::Outline1),
            ORG_OUTLINE2 => self.handle_color_button(TextType::Original, ColorType::Outline2),
            ORG_SHADOW_COLOR => self.handle_color_button(TextType::Original, ColorType::Shadow),
            ORG_FONT => self.handle_font_button(TextType::Original),
            ORG_SHADOW => {
                self.config.borrow_mut().toggle_shadow(TextType::Original);
                self.notify_change();
            }

            // TRANS 색상들
            TRANS_COLOR => self.handle_color_button(TextType::Translation, ColorType::Primary),
            TRANS_OUTLINE1 => self.handle_color_button(TextType::Translation, ColorType::Outline1),
            TRANS_OUTLINE2 => self.handle_color_button(TextType::Translation, ColorType::Outline2),
            TRANS_SHADOW_COLOR => self.handle_color_button(TextType::Translation, ColorType::Shadow),
            TRANS_FONT => self.handle_font_button(TextType::Translation),
            TRANS_SHADOW => {
                self.config.borrow_mut().toggle_shadow(TextType::Translation);
                self.notify_change();
            }

            // 테두리 설정
            BORDER_MODE => {
                self.config.borrow_mut().toggle_border_visible();
                self.notify_change();
            }
            BORDER_COLOR => {
                let initial = self.config.borrow().border_color;
                if let Some(result) = ColorDialog::show_simple(self.hwnd, initial) {
                    self.config.borrow_mut().border_color = result.argb;
                    self.notify_change();
                }
            }

            // 표시 옵션
            PRINT_ORGTEXT => {
                let mut cfg = self.config.borrow_mut();
                cfg.show_original = !cfg.show_original;
                drop(cfg);
                self.notify_change();
            }
            PRINT_TRANSTEXT => {
                let mut cfg = self.config.borrow_mut();
                cfg.show_translation = !cfg.show_translation;
                drop(cfg);
                self.notify_change();
            }
            PRINT_ORGNAME => {
                let mut cfg = self.config.borrow_mut();
                cfg.show_name = !cfg.show_name;
                drop(cfg);
                self.notify_change();
            }
            SEPERATE_NAME => {
                let mut cfg = self.config.borrow_mut();
                cfg.separate_name = !cfg.separate_name;
                drop(cfg);
                self.notify_change();
            }
            REPEAT_TEXT => {
                // 모드 순환 (0→1→2→3→4→0)
                let mut cfg = self.config.borrow_mut();
                cfg.repeat_text_mode = (cfg.repeat_text_mode + 1) % 5;
                let new_mode = cfg.repeat_text_mode;
                drop(cfg);
                // 버튼 텍스트 업데이트
                unsafe {
                    if let Ok(btn) = GetDlgItem(Some(self.hwnd), REPEAT_TEXT as i32) {
                        if !btn.is_invalid() {
                            let text = format!("반복:{}", new_mode);
                            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                            let _ = SetWindowTextW(btn, PCWSTR(text_wide.as_ptr()));
                        }
                    }
                }
                self.notify_change();
            }

            // 텍스트 정렬
            TEXTALIGN_LEFT => {
                self.config.borrow_mut().text_align = TextAlign::Left;
                self.notify_change();
            }
            TEXTALIGN_MID => {
                self.config.borrow_mut().text_align = TextAlign::Center;
                self.notify_change();
            }
            TEXTALIGN_RIGHT => {
                self.config.borrow_mut().text_align = TextAlign::Right;
                self.notify_change();
            }

            // 윈도우 옵션
            TOPMOST => {
                let mut cfg = self.config.borrow_mut();
                cfg.window_topmost = !cfg.window_topmost;
                drop(cfg);
                self.notify_change();
            }
            USE_MAGNETIC => {
                self.config.borrow_mut().toggle_magnetic_mode();
                self.notify_change();
            }
            MAGNETIC_MINIMIZE => {
                let mut cfg = self.config.borrow_mut();
                cfg.magnetic_minimize = !cfg.magnetic_minimize;
                drop(cfg);
                self.notify_change();
            }
            HIDEWIN => {
                let mut cfg = self.config.borrow_mut();
                cfg.temp_window_hide = !cfg.temp_window_hide;
                drop(cfg);
                self.notify_change();
            }
            CLIPBOARD_WATCH => {
                self.config.borrow_mut().toggle_clipboard_watch();
                self.notify_change();
            }
            WNDCLICK_THROUGH => {
                self.config.borrow_mut().toggle_click_through();
                self.notify_change();
            }

            // 텍스트 크기 +/-
            TEXTSIZE_MINUS => {
                let mut cfg = self.config.borrow_mut();
                let current = cfg.translation_style.size;
                if current > 6 {
                    cfg.set_all_text_size(ColorType::Primary, current - 1);
                }
                drop(cfg);
                self.notify_change();
            }
            TEXTSIZE_PLUS => {
                let mut cfg = self.config.borrow_mut();
                let current = cfg.translation_style.size;
                if current < 100 {
                    cfg.set_all_text_size(ColorType::Primary, current + 1);
                }
                drop(cfg);
                self.notify_change();
            }

            // 외곽선1 +/-
            OUTLINE1_MINUS => {
                let mut cfg = self.config.borrow_mut();
                let current = cfg.translation_style.outline1_size;
                if current > 0 {
                    cfg.set_all_text_size(ColorType::Outline1, current - 1);
                }
                drop(cfg);
                self.notify_change();
            }
            OUTLINE1_PLUS => {
                let mut cfg = self.config.borrow_mut();
                let current = cfg.translation_style.outline1_size;
                if current < 20 {
                    cfg.set_all_text_size(ColorType::Outline1, current + 1);
                }
                drop(cfg);
                self.notify_change();
            }

            // 외곽선2 +/-
            OUTLINE2_MINUS => {
                let mut cfg = self.config.borrow_mut();
                let current = cfg.translation_style.outline2_size;
                if current > 0 {
                    cfg.set_all_text_size(ColorType::Outline2, current - 1);
                }
                drop(cfg);
                self.notify_change();
            }
            OUTLINE2_PLUS => {
                let mut cfg = self.config.borrow_mut();
                let current = cfg.translation_style.outline2_size;
                if current < 20 {
                    cfg.set_all_text_size(ColorType::Outline2, current + 1);
                }
                drop(cfg);
                self.notify_change();
            }

            // 스크린샷 경로 찾아보기
            SCREENSHOT_PATH_BROWSE => {
                if let Some(path) = self.browse_folder() {
                    self.config.borrow_mut().screenshot.path = path.clone();
                    unsafe {
                        if let Ok(edit) = GetDlgItem(Some(self.hwnd), SCREENSHOT_PATH_EDIT as i32) {
                            if !edit.is_invalid() {
                                let text_wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
                                let _ = SetWindowTextW(edit, PCWSTR(text_wide.as_ptr()));
                            }
                        }
                    }
                    self.notify_change();
                }
            }

            _ => {}
        }
    }

    /// 색상 버튼 처리
    fn handle_color_button(&mut self, text_type: TextType, color_type: ColorType) {
        let initial = self.config.borrow().get_text_color(text_type, color_type);
        if let Some(result) = ColorDialog::show_simple(self.hwnd, initial) {
            self.config.borrow_mut().set_text_color(text_type, color_type, result.argb);
            self.notify_change();
        }
    }

    /// 폰트 버튼 처리
    fn handle_font_button(&mut self, text_type: TextType) {
        let (face, style_bits, size) = {
            let cfg = self.config.borrow();
            let style = cfg.get_text_style(text_type);
            (style.font_face.clone(), style.font_style, style.size)
        };

        let font_config = FontDialogConfig {
            initial_face: Some(face),
            initial_style: FontStyle::from_bits(style_bits),
            initial_point_size: size,
            no_activate: true,
        };

        if let Some(result) = FontDialog::show(self.hwnd, font_config) {
            let mut cfg = self.config.borrow_mut();
            let style = cfg.get_text_style_mut(text_type);
            style.font_face = result.face_name;
            style.font_style = result.style.to_bits();
            drop(cfg);
            self.notify_change();
        }
    }

    /// 트랙바 변경 처리
    fn handle_trackbar(&mut self, id: u16, value: i32) {
        use ctrl_id::*;

        match id {
            BACKGROUND_TRACKBAR => {
                let mut cfg = self.config.borrow_mut();
                let rgb = cfg.background_color & 0x00FFFFFF;
                cfg.background_color = ((value as u32) << 24) | rgb;
                drop(cfg);
                self.notify_change();
            }
            TEXTSIZE_TRACKBAR => {
                self.config.borrow_mut().set_all_text_size(ColorType::Primary, value);
                self.notify_change();
            }
            OUTLINE1_TRACKBAR => {
                self.config.borrow_mut().set_all_text_size(ColorType::Outline1, value);
                self.notify_change();
            }
            OUTLINE2_TRACKBAR => {
                self.config.borrow_mut().set_all_text_size(ColorType::Outline2, value);
                self.notify_change();
            }
            SHADOW_X_TRACKBAR => {
                self.config.borrow_mut().shadow_offset_x = value;
                self.notify_change();
            }
            SHADOW_Y_TRACKBAR => {
                self.config.borrow_mut().shadow_offset_y = value;
                self.notify_change();
            }
            MARGIN_X_TRACKBAR => {
                self.config.borrow_mut().text_margin_x = value;
                self.notify_change();
            }
            MARGIN_Y_TRACKBAR => {
                self.config.borrow_mut().text_margin_y = value;
                self.notify_change();
            }
            MARGIN_NAME_TRACKBAR => {
                self.config.borrow_mut().name_margin = value;
                self.notify_change();
            }
            BORDER_SIZE_TRACKBAR => {
                self.config.borrow_mut().border_width = value;
                self.notify_change();
            }
            SCREENSHOT_QUALITY_TRACKBAR => {
                self.config.borrow_mut().screenshot.jpeg_quality = value as u8;
                // 품질 텍스트 업데이트
                unsafe {
                    if let Ok(label) = GetDlgItem(Some(self.hwnd), SCREENSHOT_QUALITY_TEXT as i32) {
                        if !label.is_invalid() {
                            let text = format!("{}", value);
                            let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                            let _ = SetWindowTextW(label, PCWSTR(text_wide.as_ptr()));
                        }
                    }
                }
                self.notify_change();
            }
            _ => {}
        }
    }

    /// ComboBox 선택 변경 처리
    fn handle_combobox(&mut self, id: u16) {
        use ctrl_id::*;

        unsafe {
            let combo = match GetDlgItem(Some(self.hwnd), id as i32) {
                Ok(h) if !h.is_invalid() => h,
                _ => return,
            };
            let sel = SendMessageW(combo, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as usize;

            match id {
                SCREENSHOT_FORMAT => {
                    self.config.borrow_mut().screenshot.format = sel as u8;
                    self.notify_change();
                }
                SCREENSHOT_COMPRESSION => {
                    self.config.borrow_mut().screenshot.compression = sel as u8;
                    self.notify_change();
                }
                _ => {}
            }
        }
    }

    /// 폴더 브라우저 열기
    fn browse_folder(&self) -> Option<String> {
        use windows::Win32::UI::Shell::{
            IFileOpenDialog, FileOpenDialog, FOS_PICKFOLDERS, IShellItem,
            SIGDN_FILESYSPATH,
        };
        use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED};

        unsafe {
            // COM 초기화
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            let dialog: IFileOpenDialog = match CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL) {
                Ok(d) => d,
                Err(_) => {
                    CoUninitialize();
                    return None;
                }
            };

            // 폴더 선택 모드
            let _ = dialog.SetOptions(FOS_PICKFOLDERS);
            let _ = dialog.SetTitle(w!("스크린샷 저장 경로 선택"));

            // 대화상자 표시
            let result = dialog.Show(Some(self.hwnd));
            if result.is_err() {
                CoUninitialize();
                return None;
            }

            // 결과 가져오기
            let item: IShellItem = match dialog.GetResult() {
                Ok(i) => i,
                Err(_) => {
                    CoUninitialize();
                    return None;
                }
            };

            let path_ptr = match item.GetDisplayName(SIGDN_FILESYSPATH) {
                Ok(p) => p,
                Err(_) => {
                    CoUninitialize();
                    return None;
                }
            };

            // PWSTR을 String으로 변환
            let path = path_ptr.to_string().ok();

            // COM 정리
            windows::Win32::System::Com::CoTaskMemFree(Some(path_ptr.0 as *const _));
            CoUninitialize();

            path
        }
    }

    /// 설정 변경 알림
    fn notify_change(&self) {
        // 콜백 호출
        if let Some(ref cb) = self.on_change {
            cb(&self.config.borrow());
        }

        // 메인 윈도우에 WM_PAINT 전송
        unsafe {
            let _ = PostMessageW(Some(self.main_hwnd), WM_PAINT, WPARAM(0), LPARAM(1));
        }
    }

    /// WndProc
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT { unsafe {
        let instance = SETTINGS_INSTANCE.with(|cell| cell.borrow().clone());

        if let Some(dialog) = instance {
            match msg {
                WM_COMMAND => {
                    let id = (wparam.0 & 0xFFFF) as u16;
                    let notify_code = ((wparam.0 >> 16) & 0xFFFF) as u32;

                    // ComboBox 선택 변경
                    if notify_code == CBN_SELCHANGE {
                        dialog.borrow_mut().handle_combobox(id);
                    } else {
                        dialog.borrow_mut().handle_command(id);
                    }
                    return LRESULT(0);
                }

                WM_HSCROLL => {
                    // 트랙바 이벤트
                    let code = (wparam.0 & 0xFFFF) as u32;
                    let trackbar_hwnd = HWND(lparam.0 as *mut _);

                    let id = GetDlgCtrlID(trackbar_hwnd) as u16;
                    let value = match code {
                        TB_THUMBTRACK => ((wparam.0 >> 16) & 0xFFFF) as i32,
                        TB_LINEUP | TB_LINEDOWN | TB_PAGEUP | TB_PAGEDOWN | TB_TOP | TB_BOTTOM | TB_ENDTRACK => {
                            SendMessageW(trackbar_hwnd, TBM_GETPOS_VAL, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32
                        }
                        _ => return LRESULT(0),
                    };

                    dialog.borrow_mut().handle_trackbar(id, value);
                    return LRESULT(0);
                }

                WM_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }

                WM_DESTROY => {
                    // 인스턴스 정리
                    SETTINGS_INSTANCE.with(|cell| {
                        *cell.borrow_mut() = None;
                    });
                    return LRESULT(0);
                }

                WM_LBUTTONDOWN => {
                    // 창 드래그
                    let _ = SendMessageW(hwnd, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(LPARAM(0)));
                    return LRESULT(0);
                }

                _ => {}
            }
        }

        DefWindowProcW(hwnd, msg, wparam, lparam)
    }}
}
