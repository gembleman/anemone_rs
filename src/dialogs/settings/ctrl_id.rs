//! 설정 대화상자 컨트롤 ID 상수

/// `resources/settings.rc`의 설정 다이얼로그 리소스 ID.
pub const DIALOG: u16 = 100;

// 배경 설정
pub const BACKGROUND_TRACKBAR: u16 = 1001;
pub const BACKGROUND_COLOR: u16 = 1002;
pub const BACKGROUND_SWITCH: u16 = 1003;
pub const BACKGROUND_EDIT: u16 = 1004;

// 텍스트 크기
pub const TEXTSIZE_TRACKBAR: u16 = 1010;
pub const TEXTSIZE_EDIT: u16 = 1014;

// 외곽선 크기
pub const OUTLINE1_TRACKBAR: u16 = 1020;
pub const OUTLINE1_EDIT: u16 = 1023;

pub const OUTLINE2_TRACKBAR: u16 = 1030;
pub const OUTLINE2_EDIT: u16 = 1033;

// 그림자 오프셋
pub const SHADOW_X_TRACKBAR: u16 = 1040;
pub const SHADOW_X_EDIT: u16 = 1041;
pub const SHADOW_Y_TRACKBAR: u16 = 1042;
pub const SHADOW_Y_EDIT: u16 = 1043;

// 텍스트 여백
pub const MARGIN_X_TRACKBAR: u16 = 1050;
pub const MARGIN_X_EDIT: u16 = 1051;
pub const MARGIN_Y_TRACKBAR: u16 = 1052;
pub const MARGIN_Y_EDIT: u16 = 1053;
pub const MARGIN_NAME_TRACKBAR: u16 = 1054;
pub const MARGIN_NAME_EDIT: u16 = 1055;

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
pub const BORDER_SIZE_EDIT: u16 = 1133;

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
pub const CLIPBOARD_CACHE_ENABLED: u16 = 1216;
pub const CLIPBOARD_CACHE_CLEAR: u16 = 1217;
pub const CLIPBOARD_SOURCE_LANG_GUARD: u16 = 1218;

// 텍스트 정렬
pub const TEXTALIGN_LEFT: u16 = 1220;
pub const TEXTALIGN_MID: u16 = 1221;
pub const TEXTALIGN_RIGHT: u16 = 1222;

// 번역 설정
pub const TRANSLATION_GROUP: u16 = 2200;
pub const TRANS_ENGINE: u16 = 1260;
pub const TRANS_SOURCE_LANG: u16 = 1261;
pub const TRANS_TARGET_LANG: u16 = 1262;
pub const EZTRANS_DICTIONARY_EDIT: u16 = 1264;
pub const EZTRANS_DICTIONARY_BROWSE: u16 = 1265;
pub const EZTRANS_EHND_EDIT: u16 = 1266;
pub const EZTRANS_EHND_BROWSE: u16 = 1267;
pub const EZTRANS_DICTIONARY_EDIT_BTN: u16 = 1268;
pub const EZTRANS_DICTIONARY_COUNT_LABEL: u16 = 2206;
/// 평면 사전을 찾을 수 없을 때만 문구가 채워지는 경고 라벨.
pub const EZTRANS_DICTIONARY_WARNING_LABEL: u16 = 2207;
/// Ehnd 폴더를 찾을 수 없을 때만 문구가 채워지는 경고 라벨.
pub const EZTRANS_EHND_WARNING_LABEL: u16 = 2208;
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
pub const LLM_REASONING_EFFORT: u16 = 1283;
pub const LLM_SYSTEM_PROMPT_EDIT: u16 = 1284;
pub const LLM_API_KEY_VISIBLE: u16 = 1285;
pub const LLM_MAX_TOKENS_EDIT: u16 = 1286;
pub const LLM_TEMPERATURE_TRACKBAR: u16 = 1287;
pub const LLM_TEMPERATURE_EDIT: u16 = 1288;
pub const LLM_DEBOUNCE_EDIT: u16 = 1289;
pub const LLM_GLOSSARY_EDIT_BTN: u16 = 1297;
pub const LLM_GLOSSARY_COUNT_LABEL: u16 = 1298;

// 커스텀 API 선택 (1310-1319 예약)
pub const CUSTOM_API_SELECT: u16 = 1310;

// 번역 서버 (translate_server) 설정 (1320-1329 예약)
// 1320은 서버 URL 입력란이었다. 서버 URL은 config.toml에서만 관리한다.
// 1321(토큰 입력란)과 1323(마스킹 해제 체크박스)은 제거됐다. 토큰은 앱이
// 받아서 암호화해 보관하며 화면에는 값을 내보내지 않는다.
/// 서버에서 무료 API 토큰을 받아 설정에 채우는 버튼.
pub const MYS_TRANSLATER_FREE_TOKEN_BTN: u16 = 1322;
/// 토큰 보유 여부만 알리는 라벨. 값 자체는 절대 넣지 않는다.
pub const MYS_TRANSLATER_TOKEN_STATUS_LABEL: u16 = 1327;
/// 현재 API 토큰 계정의 이 기기 KST 월간 사용량.
pub const MYS_TRANSLATER_USAGE_LABEL: u16 = 1324;
/// SQLite에 저장된 현재 계정의 월간 사용량을 다시 읽는 버튼.
pub const MYS_TRANSLATER_USAGE_REFRESH_BTN: u16 = 1325;
/// 설정된 서버의 공개 유료 이용권 구매 페이지를 여는 버튼.
pub const MYS_TRANSLATER_PURCHASE_BTN: u16 = 1326;

// 단축키 설정
pub const HOTKEYS_LIST: u16 = 1400;
pub const HOTKEYS_RESET: u16 = 1401;

// 애플리케이션 정보
pub const APP_VERSION: u16 = 1500;
pub const UPDATE_CHECK_BTN: u16 = 1501;
pub const UPDATE_STATUS: u16 = 1502;
pub const UPDATE_AUTO_CHECK: u16 = 1503;
pub const UPDATE_RELEASE_PAGE: u16 = 1504;

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
/// 엔진 선택과 무관하게 번역 탭에 항상 보이는 컨트롤.
///
/// 엔진 전용 컨트롤은 여기 넣지 않는다. `EZTRANS_STATIC_IDS` 등 엔진 그룹에만
/// 등록하면 `adopt_engine_controls_into_translation_tab`이 번역 탭에 자동으로
/// 합쳐 준다.
pub const TRANSLATION_COMMON_IDS: &[u16] = &[
    2200,
    2201,
    2202,
    2203,
    TRANS_ENGINE,
    TRANS_SOURCE_LANG,
    TRANS_TARGET_LANG,
];
pub const HOTKEYS_STATIC_IDS: &[u16] = &[2304];
pub const INFO_STATIC_IDS: &[u16] = &[2400, 2401, 2402, 2403, 2404, 2405];

pub const EZTRANS_STATIC_IDS: &[u16] = &[
    2204,
    2205,
    EZTRANS_DICTIONARY_COUNT_LABEL,
    EZTRANS_DICTIONARY_WARNING_LABEL,
    EZTRANS_EHND_WARNING_LABEL,
];
pub const DEEPL_STATIC_IDS: &[u16] = &[2213, 2214, 2215, 2216];
pub const PAPAGO_STATIC_IDS: &[u16] = &[2221, 2222];
pub const LLM_STATIC_IDS: &[u16] = &[2231, 2232, 2233, 2235, 2236, 2237, 2238, 2240];
pub const CUSTOM_STATIC_IDS: &[u16] = &[2241, 2242];
pub const MYS_TRANSLATER_STATIC_IDS: &[u16] = &[
    2252,
    MYS_TRANSLATER_TOKEN_STATUS_LABEL,
    MYS_TRANSLATER_USAGE_LABEL,
];
