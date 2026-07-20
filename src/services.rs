use std::rc::Rc;
use std::time::Duration;

use crate::cache::TranslationCacheStore;
use crate::file_trans::FileTranslationSupervisor;
use crate::translation::TranslationService;
use crate::translation_ui::GuiTranslationHost;

/// GUI bootstrap에서 생성해 App과 dialog에 주입하는 장수명 서비스 집합.
pub(crate) struct AppServices {
    pub translation_ui: Rc<GuiTranslationHost>,
    pub file_translation: Rc<FileTranslationSupervisor>,
    pub translation_cache: Rc<TranslationCacheStore>,
}

impl AppServices {
    pub fn new() -> Self {
        let translation = TranslationService::new();
        let http_client = translation.http_client();
        let translation_ui = Rc::new(GuiTranslationHost::new(translation));
        let file_translation = Rc::new(FileTranslationSupervisor::with_http_client(http_client));
        let translation_cache = Rc::new(TranslationCacheStore::open(&crate::runtime::cache_db_file()));
        Self {
            translation_ui,
            file_translation,
            translation_cache,
        }
    }

    pub fn shutdown(&self) {
        self.translation_ui.shutdown();
        let report = self.file_translation.shutdown(Duration::from_secs(2));
        if report.detached > 0 {
            tracing::warn!(
                detached_tasks = report.detached,
                "file translation tasks exceeded shutdown grace"
            );
        }
    }
}
