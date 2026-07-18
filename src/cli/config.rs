use crate::config::Config;

#[derive(clap::Subcommand)]
pub(super) enum Command {
    /// 현재 config.toml 내용 출력
    Show {
        /// 비밀값 원문 출력(터미널/로그 노출 위험)
        #[arg(long)]
        reveal_secrets: bool,
    },
    /// 점 경로로 config 값 조회
    Get {
        key: String,
        /// 비밀값 원문 출력(터미널/로그 노출 위험)
        #[arg(long)]
        reveal_secret: bool,
    },
    /// 점 경로로 config 값 변경 후 저장
    Set { key: String, value: String },
    /// 비밀값을 프로세스 인자가 아닌 숨김 프롬프트/stdin으로 읽어 저장
    SetSecret { key: String },
}

pub(super) fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Show { reveal_secrets } => {
            let mut config = Config::load_or_default();
            if reveal_secrets {
                eprintln!("warning: 비밀값 원문을 출력합니다. 터미널 캡처와 로그에 주의하세요.");
            } else {
                redact_config(&mut config);
            }
            let toml =
                toml::to_string_pretty(&config).map_err(|e| format!("TOML 직렬화 실패: {e}"))?;
            print!("{toml}");
            Ok(())
        }
        Command::Get { key, reveal_secret } => {
            let config = Config::load_or_default();
            let value = if is_sensitive_key(&key) && !reveal_secret {
                "***".to_string()
            } else {
                if is_sensitive_key(&key) {
                    eprintln!(
                        "warning: 비밀값 원문을 출력합니다. 터미널 캡처와 로그에 주의하세요."
                    );
                }
                config_get(&config, &key)?
            };
            println!("{value}");
            Ok(())
        }
        Command::Set { key, value } => {
            if is_sensitive_key(&key) {
                return Err(
                    "비밀값은 명령행 인자에 둘 수 없습니다. `config set-secret <key>`를 사용하세요."
                        .to_string(),
                );
            }
            let mut config = Config::load_or_default();
            config_set(&mut config, &key, &value)?;
            config
                .save()
                .map_err(|e| format!("config 저장 실패: {e}"))?;
            println!("저장됨: {key} = {value}");
            Ok(())
        }
        Command::SetSecret { key } => {
            if !is_sensitive_key(&key) || key == "translation.deepl_keys" {
                return Err(format!("set-secret으로 설정할 수 없는 키입니다: {key}"));
            }
            let secret = read_secret()?;
            let mut config = Config::load_or_default();
            config_set(&mut config, &key, &secret)?;
            config
                .save()
                .map_err(|e| format!("config 저장 실패: {e}"))?;
            println!("저장됨: {key} = ***");
            Ok(())
        }
    }
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key,
        "translation.deepl_api_key"
            | "translation.deepl_keys"
            | "translation.papago_client_id"
            | "translation.papago_client_secret"
            | "translation.llm.api_key"
            | "translation.custom.api_key"
    ) || key == "translation.custom_apis"
        || key.starts_with("translation.custom_apis.")
}

fn redact_config(config: &mut Config) {
    config.translation.deepl_api_key = "***".to_string();
    for key in &mut config.translation.deepl_keys {
        *key = "***".to_string();
    }
    config.translation.papago_client_id = "***".to_string();
    config.translation.papago_client_secret = "***".to_string();
    config.translation.llm.api_key = "***".to_string();
    config.translation.custom.api_key = "***".to_string();
    for api in &mut config.translation.custom_apis {
        api.api_key = "***".to_string();
    }
}

fn read_secret() -> Result<String, String> {
    use std::io::{IsTerminal, Read};

    let secret = if std::io::stdin().is_terminal() {
        rpassword::prompt_password("비밀값: ").map_err(|e| format!("비밀값 입력 실패: {e}"))?
    } else {
        let mut value = String::new();
        std::io::stdin()
            .read_to_string(&mut value)
            .map_err(|e| format!("stdin 읽기 실패: {e}"))?;
        value.trim_end_matches(['\r', '\n']).to_string()
    };
    if secret.is_empty() {
        return Err("빈 비밀값은 저장하지 않습니다.".to_string());
    }
    Ok(secret)
}

pub(super) fn print_path() -> Result<(), String> {
    let path = Config::default_config_path();
    let s = path.to_string_lossy();
    println!("{s}");
    Ok(())
}

/// 점 경로로 config 값을 문자열로 조회한다. 점 경로 분기를 위해
/// `serde_json::Value` 트리로 한 번 변환한 뒤 탐색해 값을 꺼내는 단순 구현.
/// 깊은 중첩(예: `translation.llm.temperature`)도 자연스럽게 처리된다.
fn config_get(config: &Config, key: &str) -> Result<String, String> {
    let v = serde_json::to_value(config).map_err(|e| format!("config 직렬화 실패: {e}"))?;
    let mut cur = &v;
    for part in key.split('.') {
        match cur {
            serde_json::Value::Object(map) => match map.get(part) {
                Some(next) => cur = next,
                None => return Err(format!("키를 찾을 수 없습니다: {key} (segment: {part})")),
            },
            _ => return Err(format!("키 경로가 객체가 아닙니다: {key}")),
        }
    }
    Ok(match cur {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    })
}

/// 점 경로로 config 값을 설정. 직렬화→수정→역직렬화 패턴.
/// `translation.llm.glossary` 같은 배열 필드는 set 대상에서 제외(현재 CLI 범위 밖).
fn config_set(config: &mut Config, key: &str, value: &str) -> Result<(), String> {
    let mut v = serde_json::to_value(&*config).map_err(|e| format!("config 직렬화 실패: {e}"))?;
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        return Err("빈 키".to_string());
    }

    // 마지막 부분 직전까지 mutable 포인터 이동
    let (last, prefix) = parts.split_last().expect("not empty");
    let mut cur = &mut v;
    for part in prefix {
        match cur {
            serde_json::Value::Object(map) => {
                cur = map
                    .get_mut(*part)
                    .ok_or_else(|| format!("키를 찾을 수 없습니다: {key} (segment: {part})"))?;
            }
            _ => return Err(format!("키 경로가 객체가 아닙니다: {key}")),
        }
    }
    let target_obj = match cur {
        serde_json::Value::Object(map) => map,
        _ => return Err(format!("키 경로가 객체가 아닙니다: {key}")),
    };
    let slot = target_obj
        .get_mut(*last)
        .ok_or_else(|| format!("키를 찾을 수 없습니다: {key} (segment: {last})"))?;

    // 기존 값의 타입을 보존해 파싱한다. (예: bool 필드에는 "true"/"false"만 허용)
    let new_val = coerce_value(slot, value).map_err(|e| format!("값 변환 실패 ({key}): {e}"))?;
    *slot = new_val;

    let mut new_config: Config =
        serde_json::from_value(v).map_err(|e| format!("config 역직렬화 실패: {e}"))?;
    new_config.normalize();
    *config = new_config;
    Ok(())
}

fn coerce_value(existing: &serde_json::Value, raw: &str) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    match existing {
        Value::Bool(_) => match raw.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(Value::Bool(true)),
            "false" | "0" | "no" | "off" => Ok(Value::Bool(false)),
            _ => Err(format!("bool 값으로 변환할 수 없습니다: {raw}")),
        },
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                let parsed: i64 = raw
                    .parse()
                    .map_err(|_| format!("정수로 변환할 수 없습니다: {raw}"))?;
                Ok(Value::Number(parsed.into()))
            } else {
                let parsed: f64 = raw
                    .parse()
                    .map_err(|_| format!("실수로 변환할 수 없습니다: {raw}"))?;
                let num = serde_json::Number::from_f64(parsed)
                    .ok_or_else(|| "NaN/Inf 는 허용되지 않습니다".to_string())?;
                Ok(Value::Number(num))
            }
        }
        Value::String(_) => Ok(Value::String(raw.to_string())),
        Value::Null => Ok(Value::String(raw.to_string())),
        Value::Array(_) | Value::Object(_) => Err(
            "배열/객체 필드는 CLI 에서 직접 수정할 수 없습니다. config.toml 을 직접 편집하세요."
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_covers_all_configured_credentials() {
        let mut config = Config::default();
        config.translation.deepl_api_key = "deepl-secret".to_string();
        config.translation.deepl_keys = vec!["secondary-secret".to_string()];
        config.translation.papago_client_id = "client-id".to_string();
        config.translation.papago_client_secret = "papago-secret".to_string();
        config.translation.llm.api_key = "llm-secret".to_string();
        config.translation.custom.api_key = "custom-secret".to_string();
        let named_custom = crate::config::CustomApiConfig {
            name: "named".to_string(),
            api_key: "named-custom-secret".to_string(),
            ..Default::default()
        };
        config.translation.custom_apis.push(named_custom);

        redact_config(&mut config);
        let shown = toml::to_string(&config).unwrap();

        for secret in [
            "deepl-secret",
            "secondary-secret",
            "client-id",
            "papago-secret",
            "llm-secret",
            "custom-secret",
            "named-custom-secret",
        ] {
            assert!(!shown.contains(secret), "{shown}");
        }
    }

    #[test]
    fn cli_set_normalizes_llm_limits() {
        let mut config = Config::default();
        config_set(&mut config, "translation.llm.max_tokens", "999999").unwrap();
        config_set(&mut config, "translation.llm.debounce_ms", "999999").unwrap();
        config_set(&mut config, "translation.llm.temperature", "9.5").unwrap();
        config_set(&mut config, "translation.eztrans_process_count", "999999").unwrap();
        assert_eq!(config.translation.llm.max_tokens, 32_000);
        assert_eq!(config.translation.llm.debounce_ms, 10_000);
        assert_eq!(config.translation.llm.temperature, 2.0);
        assert_eq!(config.translation.eztrans_process_count, 16);
    }

    #[test]
    fn sensitive_key_registry_includes_every_credential_field() {
        for key in [
            "translation.deepl_api_key",
            "translation.deepl_keys",
            "translation.papago_client_id",
            "translation.papago_client_secret",
            "translation.llm.api_key",
            "translation.custom.api_key",
            "translation.custom_apis",
            "translation.custom_apis.0.api_key",
        ] {
            assert!(is_sensitive_key(key), "{key}");
        }
        assert!(!is_sensitive_key("translation.llm.model"));
    }
}
