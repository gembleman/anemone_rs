use super::{EzTransTranslator, Language, TranslationError, TranslationResult};
use std::path::Path;
use std::sync::{OnceLock, mpsc};

/// 무거운 32-bit EzTrans DLL 인스턴스를 하나만 유지하는 전역 관리자.
struct EzTransState {
    engine: Option<EzTransTranslator>,
    /// 현재 DLL/data 경로. 같으면 재사용하고 다르면 다시 로드한다.
    loaded_paths: Option<(String, String)>,
    /// EzTrans 인접 DLL 탐색을 위해 등록한 검색 경로.
    registered_dll_dir: Option<RegisteredDllDirectory>,
}

impl EzTransState {
    fn new() -> Self {
        Self {
            engine: None,
            loaded_paths: None,
            registered_dll_dir: None,
        }
    }

    /// EzTrans를 초기화하거나 경로가 바뀌면 다시 로드한다.
    fn init(&mut self, dll_path: &str, dat_path: &str) -> Result<(), String> {
        if let Some((loaded_dll, loaded_dat)) = &self.loaded_paths {
            if loaded_dll == dll_path && loaded_dat == dat_path && self.engine.is_some() {
                return Ok(());
            }
            self.engine = None;
            self.loaded_paths = None;
            self.registered_dll_dir = None;
        }
        let directory = RegisteredDllDirectory::register(dll_path)?;
        let engine = EzTransTranslator::new(dll_path, dat_path)?;
        self.registered_dll_dir = Some(directory);
        self.engine = Some(engine);
        self.loaded_paths = Some((dll_path.to_string(), dat_path.to_string()));
        Ok(())
    }
}

struct RegisteredDllDirectory {
    path: String,
    cookie: usize,
}

impl RegisteredDllDirectory {
    fn register(dll_path: &str) -> Result<Self, String> {
        let dir = eztrans_dll_search_dir(dll_path)?;
        let wide: Vec<u16> = dir.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is a null-terminated UTF-16 string valid for this call.
        // Windows copies the directory path into the process DLL directory list.
        unsafe {
            let cookie = windows::Win32::System::LibraryLoader::AddDllDirectory(
                windows::core::PCWSTR(wide.as_ptr()),
            );
            if cookie.is_null() {
                let err = windows::Win32::Foundation::GetLastError();
                return Err(format!("EzTrans DLL 폴더 등록 실패: Win32 {}", err.0));
            }
            Ok(Self {
                path: dir,
                cookie: cookie as usize,
            })
        }
    }
}

impl Drop for RegisteredDllDirectory {
    fn drop(&mut self) {
        unsafe {
            if let Err(error) = windows::Win32::System::LibraryLoader::RemoveDllDirectory(
                self.cookie as *const std::ffi::c_void,
            ) {
                tracing::warn!("EzTrans DLL 폴더 등록 해제 실패 ({}): {error}", self.path);
            }
        }
    }
}

fn eztrans_dll_search_dir(dll_path: &str) -> Result<String, String> {
    let path = Path::new(dll_path);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "EzTrans DLL 폴더를 확인할 수 없습니다.".to_string())?;
    let dir = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("현재 폴더 확인 실패: {e}"))?
            .join(parent)
    };
    dir.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "EzTrans DLL 폴더 경로가 UTF-8 이 아닙니다.".to_string())
}

enum EzTransCommand {
    Init {
        dll_path: String,
        dat_path: String,
        response: mpsc::Sender<Result<(), String>>,
    },
    Translate {
        text: String,
        source: Language,
        target: Language,
        response: mpsc::Sender<TranslationResult>,
    },
}

struct EzTransActor {
    sender: mpsc::Sender<EzTransCommand>,
}

impl EzTransActor {
    fn spawn() -> Self {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("anemone-eztrans".into())
            .spawn(move || {
                let mut state = EzTransState::new();
                while let Ok(command) = receiver.recv() {
                    match command {
                        EzTransCommand::Init {
                            dll_path,
                            dat_path,
                            response,
                        } => {
                            let _ = response.send(state.init(&dll_path, &dat_path));
                        }
                        EzTransCommand::Translate {
                            text,
                            source,
                            target,
                            response,
                        } => {
                            let result = state
                                .engine
                                .as_ref()
                                .ok_or(TranslationError::EngineNotInitialized("EzTrans"))
                                .and_then(|engine| engine.translate(&text, source, target));
                            let _ = response.send(result);
                        }
                    }
                }
            })
            .expect("EzTrans actor thread spawn failed");
        Self { sender }
    }

    fn init(&self, dll_path: &str, dat_path: &str) -> Result<(), String> {
        let (response, receiver) = mpsc::channel();
        self.sender
            .send(EzTransCommand::Init {
                dll_path: dll_path.to_string(),
                dat_path: dat_path.to_string(),
                response,
            })
            .map_err(|_| "EzTrans 전용 스레드가 종료되었습니다".to_string())?;
        receiver
            .recv()
            .map_err(|_| "EzTrans 초기화 응답을 받지 못했습니다".to_string())?
    }

    fn translate(&self, text: &str, source: Language, target: Language) -> TranslationResult {
        let (response, receiver) = mpsc::channel();
        self.sender
            .send(EzTransCommand::Translate {
                text: text.to_string(),
                source,
                target,
                response,
            })
            .map_err(|_| TranslationError::Engine("EzTrans 전용 스레드가 종료되었습니다".into()))?;
        receiver
            .recv()
            .map_err(|_| TranslationError::Engine("EzTrans 번역 응답을 받지 못했습니다".into()))?
    }
}

static EZTRANS_ACTOR: OnceLock<EzTransActor> = OnceLock::new();

fn eztrans_actor() -> &'static EzTransActor {
    EZTRANS_ACTOR.get_or_init(EzTransActor::spawn)
}

pub fn prepare_eztrans(dll_path: &str, dat_path: &str) -> Result<(), String> {
    eztrans_actor().init(dll_path, dat_path)
}

/// 전역 EzTrans 인스턴스로 번역하며 미초기화 시 `EngineNotInitialized`를 반환한다.
pub fn translate_with_eztrans(text: &str, source: Language, target: Language) -> TranslationResult {
    eztrans_actor().translate(text, source, target)
}
