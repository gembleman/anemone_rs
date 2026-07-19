//! 설정 대화상자 컨트롤 ID 상수

/// `resources/settings.rc`의 설정 다이얼로그 리소스 ID.
pub const DIALOG: u16 = 100;

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

pub const OUTLINE2_TRACKBAR: u16 = 1030;
pub const OUTLINE2_MINUS: u16 = 1031;
pub const OUTLINE2_PLUS: u16 = 1032;

// 그림자 오프셋
pub const SHADOW_X_TRACKBAR: u16 = 1040;
pub const SHADOW_Y_TRACKBAR: u16 = 1042;

// 텍스트 여백
pub const MARGIN_X_TRACKBAR: u16 = 1050;
pub const MARGIN_Y_TRACKBAR: u16 = 1052;
pub const MARGIN_NAME_TRACKBAR: u16 = 1054;

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

// 표시 옵션
pub const PRINT_ORGTEXT: u16 = 1200;
pub const PRINT_TRANSTEXT: u16 = 1201;
pub const PRINT_ORGNAME: u16 = 1202;
pub const SEPERATE_NAME: u16 = 1203;

// 윈도우 옵션
pub const TOPMOST: u16 = 1210;
pub const USE_MAGNETIC: u16 = 1211;
pub const MAGNETIC_MINIMIZE: u16 = 1212;
pub const CLIPBOARD_WATCH: u16 = 1214;
pub const WNDCLICK_THROUGH: u16 = 1215;

// 텍스트 정렬
pub const TEXTALIGN_LEFT: u16 = 1220;
pub const TEXTALIGN_MID: u16 = 1221;
pub const TEXTALIGN_RIGHT: u16 = 1222;

// 번역 설정
pub const TRANSLATION_GROUP: u16 = 2200;
pub const TRANS_ENGINE: u16 = 1260;
pub const TRANS_SOURCE_LANG: u16 = 1261;
pub const TRANS_TARGET_LANG: u16 = 1262;
pub const EZTRANS_DLL_EDIT: u16 = 1264;
pub const EZTRANS_DLL_BROWSE: u16 = 1265;
pub const EZTRANS_DAT_EDIT: u16 = 1266;
pub const EZTRANS_DAT_BROWSE: u16 = 1267;
// DeepL 멀티 키 (1272-1279 예약)
pub const DEEPL_KEYS_LIST: u16 = 1272;
pub const DEEPL_KEY_ADD_EDIT: u16 = 1273;
pub const DEEPL_KEY_ADD_BTN: u16 = 1274;
pub const DEEPL_KEY_REMOVE_BTN: u16 = 1275;
pub const DEEPL_STRATEGY_COMBO: u16 = 1276;
pub const DEEPL_KEY_TIER_COMBO: u16 = 1277;

pub const PAPAGO_ID_EDIT: u16 = 1270;
pub const PAPAGO_SECRET_EDIT: u16 = 1271;

// LLM 설정 (1280-1299 예약)
pub const LLM_PROVIDER: u16 = 1280;
pub const LLM_MODEL_EDIT: u16 = 1281;
pub const LLM_API_KEY_EDIT: u16 = 1282;
pub const LLM_SYSTEM_PROMPT_EDIT: u16 = 1284;
pub const LLM_MAX_TOKENS_EDIT: u16 = 1286;
pub const LLM_TEMPERATURE_TRACKBAR: u16 = 1287;
pub const LLM_TEMPERATURE_LABEL: u16 = 1288;
pub const LLM_DEBOUNCE_EDIT: u16 = 1289;
pub const LLM_GLOSSARY_EDIT_BTN: u16 = 1297;
pub const LLM_GLOSSARY_COUNT_LABEL: u16 = 1298;

// 커스텀 API 선택 (1310-1319 예약)
pub const CUSTOM_API_SELECT: u16 = 1310;

// 탭 컨트롤
pub const TAB_CONTROL: u16 = 1290;

// 적용/닫기 버튼
pub const APPLY: u16 = 1301;
pub const CLOSE: u16 = 1300;

// 리소스에서만 텍스트/그룹 박스를 식별하기 위한 ID. 탭 전환과 엔진별
// 활성화에 사용하므로 `resources/settings.rc`의 값과 동기화해야 한다.
pub const APPEARANCE_STATIC_IDS: &[u16] = &[
    2000, 2001, 2002, 2003, 2004, 2005, 2006, 2007, 2008, 2009, 2010, 2011, 2012, 2013, 2014, 2015,
    2016,
];
pub const DISPLAY_STATIC_IDS: &[u16] = &[2100, 2101];
pub const TRANSLATION_STATIC_IDS: &[u16] = &[
    2200, 2201, 2202, 2203, 2204, 2205, 2213, 2214, 2215, 2216, 2221, 2222, 2231, 2232, 2233, 2235,
    2236, 2237, 2238, 2239, 2241, 2242,
];

pub const EZTRANS_STATIC_IDS: &[u16] = &[2204, 2205];
pub const DEEPL_STATIC_IDS: &[u16] = &[2213, 2214, 2215, 2216];
pub const PAPAGO_STATIC_IDS: &[u16] = &[2221, 2222];
pub const LLM_STATIC_IDS: &[u16] = &[2231, 2232, 2233, 2235, 2236, 2237, 2238, 2239];
pub const CUSTOM_STATIC_IDS: &[u16] = &[2241, 2242];
