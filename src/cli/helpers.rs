/// 최소한의 JSON 출력 헬퍼. 외부 의존성 없이 짧은 객체만 출력하므로
/// 문자열 이스케이프만 직접 구현한다.
pub(super) enum JsonVal<'a> {
    Str(&'a str),
    Bool(bool),
    Num(i64),
}

pub(super) fn json_object(entries: &[(&str, JsonVal<'_>)]) -> String {
    let mut s = String::from("{");
    for (i, (k, v)) in entries.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('"');
        s.push_str(k);
        s.push_str("\":");
        match v {
            JsonVal::Str(value) => {
                s.push('"');
                json_escape_into(&mut s, value);
                s.push('"');
            }
            JsonVal::Bool(b) => s.push_str(if *b { "true" } else { "false" }),
            JsonVal::Num(n) => s.push_str(&n.to_string()),
        }
    }
    s.push('}');
    s
}

fn json_escape_into(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
}
