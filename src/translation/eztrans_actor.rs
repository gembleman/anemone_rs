use super::{EzTransTranslator, Language, TranslationError, TranslationResult};
use std::sync::{Mutex, OnceLock, mpsc};

/// EHND 초기화는 프로세스 경계를 넘어 같은 자산을 읽는다. GUI in-process
/// 로드(`EzTransState::init`)와 파일 번역 helper 프로세스 풀
/// (`eztrans_process::worker_loop`)이 동시에 초기화하면 간헐적 시작 실패가
/// 난다 — 초기화 구간을 프로세스 전역으로 직렬화한다.
static EZTRANS_INITIALIZATION_GATE: OnceLock<Mutex<()>> = OnceLock::new();

/// GUI actor와 파일 번역 풀이 함께 쓰는 전역 초기화 직렬화 게이트.
pub(super) fn eztrans_initialization_gate() -> &'static Mutex<()> {
    EZTRANS_INITIALIZATION_GATE.get_or_init(|| Mutex::new(()))
}

/// 무거운 EzTrans 세션 인스턴스를 하나만 유지하는 전역 관리자.
struct EzTransState {
    engine: Option<EzTransTranslator>,
    /// 현재 사전/Ehnd 경로. 같으면 재사용하고 다르면 다시 로드한다.
    loaded_paths: Option<(String, String)>,
}

impl EzTransState {
    fn new() -> Self {
        Self {
            engine: None,
            loaded_paths: None,
        }
    }

    /// EzTrans를 초기화하거나 경로가 바뀌면 다시 로드한다.
    fn init(&mut self, dictionary_path: &str, ehnd_path: &str) -> Result<(), String> {
        if let Some((loaded_dictionary, loaded_ehnd)) = &self.loaded_paths {
            if loaded_dictionary == dictionary_path
                && loaded_ehnd == ehnd_path
                && self.engine.is_some()
            {
                return Ok(());
            }
            self.engine = None;
            self.loaded_paths = None;
        }
        // GUI와 helper 풀이 동시에 큰 자산을 로드하지 않도록 직렬화한다.
        let _gate = eztrans_initialization_gate()
            .lock()
            .map_err(|_| "EzTrans 초기화 잠금이 손상되었습니다".to_string())?;
        let engine = EzTransTranslator::new(dictionary_path, ehnd_path)?;
        self.engine = Some(engine);
        self.loaded_paths = Some((dictionary_path.to_string(), ehnd_path.to_string()));
        Ok(())
    }
}

enum EzTransCommand {
    Init {
        dictionary_path: String,
        ehnd_path: String,
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
                            dictionary_path,
                            ehnd_path,
                            response,
                        } => {
                            let _ = response.send(state.init(&dictionary_path, &ehnd_path));
                        }
                        EzTransCommand::Translate {
                            text,
                            source,
                            target,
                            response,
                        } => {
                            let result = state
                                .engine
                                .as_mut()
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

    fn init(&self, dictionary_path: &str, ehnd_path: &str) -> Result<(), String> {
        let (response, receiver) = mpsc::channel();
        self.sender
            .send(EzTransCommand::Init {
                dictionary_path: dictionary_path.to_string(),
                ehnd_path: ehnd_path.to_string(),
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

pub fn prepare_eztrans(dictionary_path: &str, ehnd_path: &str) -> Result<(), String> {
    eztrans_actor().init(dictionary_path, ehnd_path)
}

/// 전역 EzTrans 인스턴스로 번역하며 미초기화 시 `EngineNotInitialized`를 반환한다.
pub fn translate_with_eztrans(text: &str, source: Language, target: Language) -> TranslationResult {
    eztrans_actor().translate(text, source, target)
}
