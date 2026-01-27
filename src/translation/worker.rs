//! 번역 워커 스레드
//!
//! 별도 스레드에서 tokio 런타임을 실행하여 async 번역을 수행합니다.
//! UI 스레드를 블로킹하지 않고 Windows 메시지로 결과를 전달합니다.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use isolang::Language;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::Win32::Foundation::{WPARAM, LPARAM};

use super::{TranslationEngine, TranslationResult};

/// 번역 완료 메시지 ID
pub const WM_TRANSLATION_COMPLETE: u32 = 0x0400 + 100; // WM_USER + 100

/// 번역 요청
#[derive(Debug, Clone)]
pub struct TranslationRequest {
    /// 요청 ID (응답과 매칭용)
    pub id: u64,
    /// 번역할 텍스트
    pub text: String,
    /// 번역 엔진
    pub engine: TranslationEngine,
    /// 소스 언어
    pub source_lang: Language,
    /// 타겟 언어
    pub target_lang: Language,
    /// DeepL API 키 (DeepL 엔진 사용 시)
    pub deepl_api_key: Option<String>,
}

/// 번역 응답
#[derive(Debug, Clone)]
pub struct TranslationResponse {
    /// 요청 ID
    pub id: u64,
    /// 번역 결과
    pub result: TranslationResult,
}

/// 응답 저장소 (thread-safe)
static RESPONSE_STORAGE: std::sync::OnceLock<Arc<Mutex<Vec<TranslationResponse>>>> =
    std::sync::OnceLock::new();

fn get_response_storage() -> Arc<Mutex<Vec<TranslationResponse>>> {
    RESPONSE_STORAGE
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

/// 응답 저장
pub fn store_response(response: TranslationResponse) {
    if let Ok(mut storage) = get_response_storage().lock() {
        storage.push(response);
    }
}

/// 특정 ID의 응답 가져오기
pub fn take_response(id: u64) -> Option<TranslationResponse> {
    if let Ok(mut storage) = get_response_storage().lock() {
        if let Some(pos) = storage.iter().position(|r| r.id == id) {
            return Some(storage.remove(pos));
        }
    }
    None
}

/// 모든 응답 가져오기
pub fn take_all_responses() -> Vec<TranslationResponse> {
    if let Ok(mut storage) = get_response_storage().lock() {
        std::mem::take(&mut *storage)
    } else {
        Vec::new()
    }
}

/// 번역 워커
pub struct TranslationWorker {
    sender: Sender<TranslationRequest>,
    #[allow(dead_code)]
    handle: JoinHandle<()>,
    next_id: u64,
}

impl TranslationWorker {
    /// 새 워커 생성 및 시작
    pub fn spawn(hwnd: HWND) -> Self {
        let (tx, rx) = mpsc::channel::<TranslationRequest>();

        // HWND를 usize로 변환하여 Send 트레이트 문제 해결
        let hwnd_raw = hwnd.0 as usize;

        let handle = thread::spawn(move || {
            Self::worker_thread(rx, hwnd_raw);
        });

        Self {
            sender: tx,
            handle,
            next_id: 1,
        }
    }

    /// 워커 스레드 메인 함수
    fn worker_thread(rx: Receiver<TranslationRequest>, hwnd_raw: usize) {
        // usize를 HWND로 다시 변환
        let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
        // tokio 런타임 생성
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("Failed to create tokio runtime: {}", e);
                return;
            }
        };

        rt.block_on(async {
            while let Ok(req) = rx.recv() {
                let result = Self::translate_async(&req).await;

                let response = TranslationResponse {
                    id: req.id,
                    result,
                };

                // 응답 저장
                store_response(response);

                // UI 스레드에 완료 알림
                unsafe {
                    let _ = PostMessageW(
                        Some(hwnd),
                        WM_TRANSLATION_COMPLETE,
                        WPARAM(req.id as usize),
                        LPARAM(0),
                    );
                }
            }
        });
    }

    /// 비동기 번역 수행
    async fn translate_async(req: &TranslationRequest) -> TranslationResult {
        match req.engine {
            TranslationEngine::EzTrans => {
                // EzTrans는 동기 함수이므로 별도 처리
                // tokio::task::spawn_blocking 사용
                let text = req.text.clone();
                let source = req.source_lang;
                let target = req.target_lang;

                match tokio::task::spawn_blocking(move || {
                    super::translate_with_eztrans(&text, source, target)
                })
                .await
                {
                    Ok(result) => result,
                    Err(e) => TranslationResult::Error(format!("EzTrans 실행 오류: {}", e)),
                }
            }
            TranslationEngine::Google => {
                super::google::translate_async(&req.text, req.source_lang, req.target_lang).await
            }
            TranslationEngine::DeepL => {
                let api_key = req.deepl_api_key.as_deref().unwrap_or("");
                super::deepl::translate_async(&req.text, req.source_lang, req.target_lang, api_key)
                    .await
            }
        }
    }

    /// 번역 요청 전송
    pub fn request(&mut self, req: TranslationRequest) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        let mut request = req;
        request.id = id;

        if let Err(e) = self.sender.send(request) {
            eprintln!("Failed to send translation request: {}", e);
        }

        id
    }

    /// 간편 요청 메서드
    pub fn translate(
        &mut self,
        text: String,
        engine: TranslationEngine,
        source_lang: Language,
        target_lang: Language,
        deepl_api_key: Option<String>,
    ) -> u64 {
        self.request(TranslationRequest {
            id: 0, // request()에서 할당
            text,
            engine,
            source_lang,
            target_lang,
            deepl_api_key,
        })
    }
}
