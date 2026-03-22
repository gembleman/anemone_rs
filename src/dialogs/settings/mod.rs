//! 설정 대화상자
//!
//! 모든 설정을 관리하는 메인 설정 대화상자.
//! Win32 CreateWindowEx를 사용하여 컨트롤을 프로그래매틱하게 생성.

mod controls;
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

// TrackBar 메시지 상수 (windows crate 0.62에서 누락)
const TBM_GETPOS_VAL: u32 = 1024;

// ComboBox 메시지 상수
const CB_ADDSTRING: u32 = 0x0143;
const CB_SETCURSEL: u32 = 0x014E;
const CB_GETCURSEL: u32 = 0x0147;
const CBN_SELCHANGE: u32 = 1;

const SETTINGS_CLASS_NAME: PCWSTR = w!("AnemoneSettingsClass");
const SETTINGS_WIDTH: i32 = 500;
const SETTINGS_HEIGHT: i32 = 940;

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
    ) -> Result<HWND> {
        unsafe {
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
        }
    }

    /// 컨트롤 생성
    fn create_controls(&mut self) -> Result<()> {
        unsafe {
            let _hinst = GetModuleHandleW(None)?;
            let _hfont = GetStockObject(DEFAULT_GUI_FONT);

            // ====== 배경 설정 그룹 ======
            self.create_group_box(10, 10, 230, 80, "배경 설정")?;

            // 배경 투명도 트랙바
            self.create_label(20, 30, 60, 18, "투명도:")?;
            let trackbar =
                self.create_trackbar(80, 28, 150, 22, ctrl_id::BACKGROUND_TRACKBAR, 0, 255)?;
            let alpha = (self.config.borrow().background_color >> 24) & 0xFF;
            let _ = SendMessageW(
                trackbar,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(alpha as isize)),
            );

            // 배경 색상 버튼
            self.create_color_button(20, 55, 70, 25, ctrl_id::BACKGROUND_COLOR, "배경색")?;

            // 배경 표시 체크박스
            let bg_visible = self.config.borrow().background_visible;
            self.create_checkbox(
                100,
                57,
                80,
                20,
                ctrl_id::BACKGROUND_SWITCH,
                "표시",
                bg_visible,
            )?;

            // ====== 텍스트 크기 그룹 ======
            self.create_group_box(250, 10, 230, 80, "텍스트 크기")?;

            let size_trackbar =
                self.create_trackbar(260, 30, 180, 22, ctrl_id::TEXTSIZE_TRACKBAR, 6, 100)?;
            let text_size = self.config.borrow().translation_style.size;
            let _ = SendMessageW(
                size_trackbar,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(text_size as isize)),
            );

            self.create_button(260, 55, 30, 25, ctrl_id::TEXTSIZE_MINUS, "-")?;
            self.create_button(295, 55, 30, 25, ctrl_id::TEXTSIZE_PLUS, "+")?;
            self.create_label_with_id(
                330,
                58,
                100,
                18,
                ctrl_id::TEXTSIZE_TEXT,
                &format!("크기: {}", text_size),
            )?;

            // ====== 외곽선 설정 그룹 ======
            self.create_group_box(10, 95, 470, 80, "외곽선 설정")?;

            self.create_label(20, 115, 60, 18, "외곽선1:")?;
            let outline1_tb =
                self.create_trackbar(80, 113, 120, 22, ctrl_id::OUTLINE1_TRACKBAR, 0, 20)?;
            let outline1_size = self.config.borrow().translation_style.outline1_size;
            let _ = SendMessageW(
                outline1_tb,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(outline1_size as isize)),
            );
            self.create_button(205, 113, 25, 22, ctrl_id::OUTLINE1_MINUS, "-")?;
            self.create_button(232, 113, 25, 22, ctrl_id::OUTLINE1_PLUS, "+")?;

            self.create_label(270, 115, 60, 18, "외곽선2:")?;
            let outline2_tb =
                self.create_trackbar(330, 113, 120, 22, ctrl_id::OUTLINE2_TRACKBAR, 0, 20)?;
            let outline2_size = self.config.borrow().translation_style.outline2_size;
            let _ = SendMessageW(
                outline2_tb,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(outline2_size as isize)),
            );
            self.create_button(455, 113, 25, 22, ctrl_id::OUTLINE2_MINUS, "-")?;
            self.create_button(455, 140, 25, 22, ctrl_id::OUTLINE2_PLUS, "+")?;

            // 그림자 오프셋
            self.create_label(20, 145, 80, 18, "그림자 X:")?;
            let shadow_x_tb =
                self.create_trackbar(100, 143, 100, 22, ctrl_id::SHADOW_X_TRACKBAR, 0, 20)?;
            let shadow_x = self.config.borrow().shadow_offset_x;
            let _ = SendMessageW(
                shadow_x_tb,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(shadow_x as isize)),
            );

            self.create_label(220, 145, 80, 18, "그림자 Y:")?;
            let shadow_y_tb =
                self.create_trackbar(300, 143, 100, 22, ctrl_id::SHADOW_Y_TRACKBAR, 0, 20)?;
            let shadow_y = self.config.borrow().shadow_offset_y;
            let _ = SendMessageW(
                shadow_y_tb,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(shadow_y as isize)),
            );

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
            self.create_group_box(330, 180, 150, 130, "번역문 설정")?;
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
            let margin_x_tb =
                self.create_trackbar(70, 333, 100, 22, ctrl_id::MARGIN_X_TRACKBAR, 0, 300)?;
            let margin_x = self.config.borrow().text_margin_x;
            let _ = SendMessageW(
                margin_x_tb,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(margin_x as isize)),
            );

            self.create_label(180, 335, 50, 18, "상하:")?;
            let margin_y_tb =
                self.create_trackbar(230, 333, 100, 22, ctrl_id::MARGIN_Y_TRACKBAR, 0, 300)?;
            let margin_y = self.config.borrow().text_margin_y;
            let _ = SendMessageW(
                margin_y_tb,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(margin_y as isize)),
            );

            self.create_label(340, 335, 50, 18, "이름:")?;
            let margin_name_tb =
                self.create_trackbar(390, 333, 80, 22, ctrl_id::MARGIN_NAME_TRACKBAR, 0, 300)?;
            let margin_name = self.config.borrow().name_margin;
            let _ = SendMessageW(
                margin_name_tb,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(margin_name as isize)),
            );

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
            let border_tb =
                self.create_trackbar(240, 505, 150, 22, ctrl_id::BORDER_SIZE_TRACKBAR, 0, 10)?;
            let border_size = self.config.borrow().border_width;
            let _ = SendMessageW(
                border_tb,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(border_size as isize)),
            );

            // ====== 번역 설정 그룹 ======
            self.create_group_box(10, 550, 470, 120, "번역 설정")?;

            self.create_label(20, 570, 40, 18, "엔진:")?;
            let engine_items = vec!["EzTrans", "Google", "DeepL"];
            let engine_sel = self.config.borrow().translation.engine_as_u8() as usize;
            self.create_combobox(60, 568, 85, 100, ctrl_id::TRANS_ENGINE, &engine_items, engine_sel)?;

            self.create_label(155, 570, 40, 18, "소스:")?;
            let lang_items = vec!["일본어", "한국어", "영어", "중국어"];
            let config = self.config.borrow();
            let engine = config.translation.get_engine();
            let source_sel = config.translation.source_lang_index(engine);
            drop(config);
            self.create_combobox(195, 568, 80, 100, ctrl_id::TRANS_SOURCE_LANG, &lang_items, source_sel)?;

            self.create_label(285, 570, 40, 18, "타겟:")?;
            let config = self.config.borrow();
            let target_sel = config.translation.target_lang_index(engine);
            drop(config);
            self.create_combobox(325, 568, 80, 100, ctrl_id::TRANS_TARGET_LANG, &lang_items, target_sel)?;

            let auto_detect = self.config.borrow().translation.auto_detect;
            self.create_checkbox(415, 570, 60, 18, ctrl_id::TRANS_AUTO_DETECT, "자동", auto_detect)?;

            self.create_label(20, 595, 70, 18, "EzTrans DLL:")?;
            let dll_path = self.config.borrow().translation.eztrans_dll_path.clone();
            self.create_edit(90, 593, 310, 22, ctrl_id::EZTRANS_DLL_EDIT, &dll_path)?;
            self.create_button(405, 593, 65, 22, ctrl_id::EZTRANS_DLL_BROWSE, "찾아보기")?;

            self.create_label(20, 620, 70, 18, "EzTrans Dat:")?;
            let dat_path = self.config.borrow().translation.eztrans_dat_path.clone();
            self.create_edit(90, 618, 310, 22, ctrl_id::EZTRANS_DAT_EDIT, &dat_path)?;
            self.create_button(405, 618, 65, 22, ctrl_id::EZTRANS_DAT_BROWSE, "찾아보기")?;

            // ====== 닫기 버튼 ======
            self.create_button(380, 745, 100, 30, ctrl_id::CLOSE, "닫기")?;

            Ok(())
        }
    }

    /// WndProc
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe {
            let instance = SETTINGS_INSTANCE.with(|cell| cell.borrow().clone());

            if let Some(dialog) = instance {
                match msg {
                    WM_COMMAND => {
                        let id = (wparam.0 & 0xFFFF) as u16;
                        let notify_code = ((wparam.0 >> 16) & 0xFFFF) as u32;

                        if notify_code == CBN_SELCHANGE {
                            dialog.borrow_mut().handle_combobox(id);
                        } else {
                            dialog.borrow_mut().handle_command(id);
                        }
                        return LRESULT(0);
                    }

                    WM_HSCROLL => {
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
                                )
                                .0 as i32
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
                        SETTINGS_INSTANCE.with(|cell| {
                            *cell.borrow_mut() = None;
                        });
                        return LRESULT(0);
                    }

                    WM_LBUTTONDOWN => {
                        let _ = SendMessageW(
                            hwnd,
                            WM_NCLBUTTONDOWN,
                            Some(WPARAM(HTCAPTION as usize)),
                            Some(LPARAM(0)),
                        );
                        return LRESULT(0);
                    }

                    _ => {}
                }
            }

            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}
