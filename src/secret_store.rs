//! 설치형 모드의 DPAPI 비밀 저장소와 portable 평문 정책.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use windows::Win32::{
    Foundation::{HLOCAL, LocalFree},
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
};
use windows::core::w;

use crate::config::Config;
use crate::runtime::{AppMode, AppPaths};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConfigSecrets {
    deepl_api_key: String,
    deepl_keys: Vec<String>,
    papago_client_secret: String,
    llm_api_key: String,
    legacy_custom_api_key: String,
    custom_api_keys: Vec<NamedApiSecret>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct NamedApiSecret {
    name: String,
    api_key: String,
}

impl ConfigSecrets {
    pub(crate) fn capture(config: &Config) -> Self {
        Self {
            deepl_api_key: config.translation.deepl_api_key.clone(),
            deepl_keys: config.translation.deepl_keys.clone(),
            papago_client_secret: config.translation.papago_client_secret.clone(),
            llm_api_key: config.translation.llm.api_key.clone(),
            legacy_custom_api_key: config.translation.custom.api_key.clone(),
            custom_api_keys: config
                .translation
                .custom_apis
                .iter()
                .map(|api| NamedApiSecret {
                    name: api.name.clone(),
                    api_key: api.api_key.clone(),
                })
                .collect(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.deepl_api_key.is_empty()
            && self.deepl_keys.iter().all(String::is_empty)
            && self.papago_client_secret.is_empty()
            && self.llm_api_key.is_empty()
            && self.legacy_custom_api_key.is_empty()
            && self
                .custom_api_keys
                .iter()
                .all(|item| item.api_key.is_empty())
    }

    pub(crate) fn redact(config: &mut Config) {
        config.translation.deepl_api_key.clear();
        config.translation.deepl_keys.clear();
        config.translation.papago_client_secret.clear();
        config.translation.llm.api_key.clear();
        config.translation.custom.api_key.clear();
        for api in &mut config.translation.custom_apis {
            api.api_key.clear();
        }
    }

    pub(crate) fn apply_to(&self, config: &mut Config) {
        config
            .translation
            .deepl_api_key
            .clone_from(&self.deepl_api_key);
        config.translation.deepl_keys.clone_from(&self.deepl_keys);
        config
            .translation
            .papago_client_secret
            .clone_from(&self.papago_client_secret);
        config.translation.llm.api_key.clone_from(&self.llm_api_key);
        config
            .translation
            .custom
            .api_key
            .clone_from(&self.legacy_custom_api_key);
        for api in &mut config.translation.custom_apis {
            if let Some(secret) = self
                .custom_api_keys
                .iter()
                .find(|secret| secret.name == api.name)
            {
                api.api_key.clone_from(&secret.api_key);
            }
        }
    }
}

pub(crate) trait SecretStore {
    fn protects_config(&self) -> bool;
    fn load(&self) -> Result<Option<ConfigSecrets>, SecretStoreError>;
    fn save(&self, secrets: &ConfigSecrets) -> Result<(), SecretStoreError>;
}

pub(crate) enum ActiveSecretStore {
    Portable(PlaintextSecretStore),
    Installed(DpapiSecretStore),
}

impl ActiveSecretStore {
    pub(crate) fn for_paths(paths: &AppPaths) -> Self {
        match paths.mode() {
            AppMode::Portable => Self::Portable(PlaintextSecretStore),
            AppMode::Installed => Self::Installed(DpapiSecretStore {
                path: paths.secrets_file(),
            }),
        }
    }
}

impl SecretStore for ActiveSecretStore {
    fn protects_config(&self) -> bool {
        matches!(self, Self::Installed(_))
    }

    fn load(&self) -> Result<Option<ConfigSecrets>, SecretStoreError> {
        match self {
            Self::Portable(store) => store.load(),
            Self::Installed(store) => store.load(),
        }
    }

    fn save(&self, secrets: &ConfigSecrets) -> Result<(), SecretStoreError> {
        match self {
            Self::Portable(store) => store.save(secrets),
            Self::Installed(store) => store.save(secrets),
        }
    }
}

pub(crate) struct PlaintextSecretStore;

impl SecretStore for PlaintextSecretStore {
    fn protects_config(&self) -> bool {
        false
    }

    fn load(&self) -> Result<Option<ConfigSecrets>, SecretStoreError> {
        Ok(None)
    }

    fn save(&self, _secrets: &ConfigSecrets) -> Result<(), SecretStoreError> {
        Ok(())
    }
}

pub(crate) struct DpapiSecretStore {
    path: PathBuf,
}

impl SecretStore for DpapiSecretStore {
    fn protects_config(&self) -> bool {
        true
    }

    fn load(&self) -> Result<Option<ConfigSecrets>, SecretStoreError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let encrypted = std::fs::read(&self.path).map_err(|source| SecretStoreError::Read {
            path: self.path.clone(),
            source,
        })?;
        let mut plaintext = unprotect(&encrypted)?;
        let result = serde_json::from_slice(&plaintext).map_err(SecretStoreError::Decode);
        plaintext.fill(0);
        result.map(Some)
    }

    fn save(&self, secrets: &ConfigSecrets) -> Result<(), SecretStoreError> {
        let mut plaintext = serde_json::to_vec(secrets).map_err(SecretStoreError::Encode)?;
        let encrypted = protect(&plaintext);
        plaintext.fill(0);
        let encrypted = encrypted?;
        crate::fs_util::atomic_write(&self.path, &encrypted).map_err(|source| {
            SecretStoreError::Write {
                path: self.path.clone(),
                source,
            }
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SecretStoreError {
    #[error("비밀 저장소 파일 읽기 실패: {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("비밀 저장소 파일 쓰기 실패: {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("비밀 데이터 직렬화 실패: {0}")]
    Encode(serde_json::Error),
    #[error("비밀 데이터 역직렬화 실패: {0}")]
    Decode(serde_json::Error),
    #[error("DPAPI 암호화 실패: {0}")]
    Protect(windows::core::Error),
    #[error("DPAPI 복호화 실패: {0}")]
    Unprotect(windows::core::Error),
    #[error("DPAPI 입력이 너무 큽니다")]
    InputTooLarge,
}

fn protect(plaintext: &[u8]) -> Result<Vec<u8>, SecretStoreError> {
    let cb_data = u32::try_from(plaintext.len()).map_err(|_| SecretStoreError::InputTooLarge)?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: cb_data,
        pbData: plaintext.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: input slice는 호출 동안 유효하고 output은 DPAPI가 LocalAlloc로 할당한다.
    unsafe {
        CryptProtectData(
            &input,
            w!("Anemone secrets"),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(SecretStoreError::Protect)?;
    }
    copy_and_free(output)
}

fn unprotect(encrypted: &[u8]) -> Result<Vec<u8>, SecretStoreError> {
    let cb_data = u32::try_from(encrypted.len()).map_err(|_| SecretStoreError::InputTooLarge)?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: cb_data,
        pbData: encrypted.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: input slice는 호출 동안 유효하고 output은 DPAPI가 LocalAlloc로 할당한다.
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(SecretStoreError::Unprotect)?;
    }
    copy_and_free(output)
}

fn copy_and_free(output: CRYPT_INTEGER_BLOB) -> Result<Vec<u8>, SecretStoreError> {
    if output.pbData.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: DPAPI가 cbData 길이로 할당한 buffer를 복사한 뒤 LocalFree로 해제한다.
    let bytes = unsafe {
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        bytes
    };
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_redacts_and_reapplies_config_secrets() {
        let mut config = Config::default();
        config.translation.deepl_api_key = "deepl".into();
        config.translation.papago_client_secret = "papago".into();
        config.translation.llm.api_key = "llm".into();
        config.translation.custom.api_key = "custom".into();

        let secrets = ConfigSecrets::capture(&config);
        ConfigSecrets::redact(&mut config);
        assert!(ConfigSecrets::capture(&config).is_empty());

        secrets.apply_to(&mut config);
        assert_eq!(ConfigSecrets::capture(&config), secrets);
    }

    #[test]
    fn dpapi_round_trip_is_bound_to_the_current_windows_user() {
        let plaintext = b"anemone-secret-round-trip";
        let encrypted = protect(plaintext).unwrap();
        assert_ne!(encrypted, plaintext);
        assert_eq!(unprotect(&encrypted).unwrap(), plaintext);
    }

    #[test]
    fn dpapi_file_store_never_writes_plaintext_secret() {
        let root = std::env::temp_dir().join(format!(
            "anemone-secrets-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("secrets.dat");
        let store = DpapiSecretStore { path: path.clone() };
        let mut config = Config::default();
        config.translation.llm.api_key = "never-write-this-plaintext".into();
        let secrets = ConfigSecrets::capture(&config);

        store.save(&secrets).unwrap();
        let raw = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("never-write-this-plaintext"));
        assert_eq!(store.load().unwrap(), Some(secrets));
        std::fs::remove_dir_all(root).unwrap();
    }
}
