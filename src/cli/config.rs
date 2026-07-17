use crate::config::Config;

use super::helpers::{JsonVal, json_object};

#[derive(clap::Subcommand)]
pub(super) enum Command {
    /// 현재 config.toml 내용 출력
    Show,
    /// 점 경로로 config 값 조회
    Get { key: String },
    /// 점 경로로 config 값 변경 후 저장
    Set { key: String, value: String },
}

pub(super) fn run(command: Command, json: bool) -> Result<(), String> {
    match command {
        Command::Show => {
            let config = Config::load_or_default();
            let toml =
                toml::to_string_pretty(&config).map_err(|e| format!("TOML 직렬화 실패: {e}"))?;
            if json {
                let json_text =
                    serde_json::to_string(&config).map_err(|e| format!("JSON 직렬화 실패: {e}"))?;
                println!("{json_text}");
            } else {
                print!("{toml}");
            }
            Ok(())
        }
        Command::Get { key } => {
            let config = Config::load_or_default();
            let value = config_get(&config, &key)?;
            if json {
                println!(
                    "{}",
                    json_object(&[("key", JsonVal::Str(&key)), ("value", JsonVal::Str(&value))])
                );
            } else {
                println!("{value}");
            }
            Ok(())
        }
        Command::Set { key, value } => {
            let mut config = Config::load_or_default();
            config_set(&mut config, &key, &value)?;
            config
                .save()
                .map_err(|e| format!("config 저장 실패: {e}"))?;
            if json {
                println!(
                    "{}",
                    json_object(&[
                        ("key", JsonVal::Str(&key)),
                        ("value", JsonVal::Str(&value)),
                        ("saved", JsonVal::Bool(true)),
                    ])
                );
            } else {
                println!("저장됨: {key} = {value}");
            }
            Ok(())
        }
    }
}

pub(super) fn print_path(json: bool) -> Result<(), String> {
    let path = Config::default_config_path();
    let s = path.to_string_lossy();
    if json {
        println!("{}", json_object(&[("path", JsonVal::Str(&s))]));
    } else {
        println!("{s}");
    }
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

    let new_config: Config =
        serde_json::from_value(v).map_err(|e| format!("config 역직렬화 실패: {e}"))?;
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
