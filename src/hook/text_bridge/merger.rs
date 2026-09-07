//! 같은 출처(`source`)의 연속 텍스트 이벤트를 짧은 시간 창 안에서 한
//! 문장으로 병합한다.

use std::cmp::Reverse;
use std::collections::hash_map::Entry;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::time::{Duration, Instant};

/// LunaHost의 `ThreadParam(addr, ctx, ctx2)`에 대응하는 텍스트 스레드 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HookSource {
    pub address: u64,
    pub context: u64,
    pub subcontext: u64,
}

/// 후킹 텍스트 이벤트 하나가 UI로 넘어갈 준비가 된 형태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookText {
    pub source: HookSource,
    /// DLL이 보고한 후크 이름. 후킹 관리 창의 스레드 식별에 쓴다.
    pub hook_name: String,
    pub text: String,
    /// `HookType::FULL_STRING`이면 이벤트 하나가 문장 하나다. 그렇지 않으면
    /// 이벤트가 이어 붙일 글자/조각이다.
    pub full_string: bool,
}

/// 원본 LunaHost의 `TextThread`처럼 출처별로 payload를 이어 붙인다. FULL_STRING
/// 훅이 낸 문장 사이에는 줄바꿈을 넣는다.
///
/// 방출 조건은 훅 종류를 가리지 않는다 — 마지막 조각 이후 창이 지나면 낸다.
/// 원본 `TextThread::Flush`가 그렇다. 대사의 의미론적 경계는 판단하지 않는다.
///
/// 창을 재는 기준은 **파이프 수신 시각**이다. UI 메시지 큐가 밀려 여러
/// 이벤트를 한꺼번에 꺼내도 병합 결과가 달라지지 않아야 한다.
#[derive(Debug, Default)]
pub struct TextMerger {
    window_ms: u64,
    pending: HashMap<HookSource, PendingText>,
    /// 각 source의 최신 만료 시각. 갱신 때 이전 항목은 heap에 남지만
    /// `last_seen`과 비교해 폐기하므로 10ms tick마다 문자열/전체 map을
    /// 훑지 않는다.
    deadlines: BinaryHeap<(Reverse<Instant>, HookSource)>,
    over_limit: HashSet<HookSource>,
}

/// 원본 `TextThread::maxBufferSize`다. 쉬지 않고 뱉는 출처는 창이 지나지 않아
/// 영영 쌓이기만 하므로 길이로도 끊는다.
const MAX_BUFFER_CHARS: usize = 3000;

#[derive(Debug)]
struct PendingText {
    text: String,
    hook_name: String,
    last_seen: Instant,
    full_string: bool,
    char_count: usize,
}

impl PendingText {
    fn into_hook_text(self, source: HookSource) -> HookText {
        HookText {
            source,
            hook_name: self.hook_name,
            text: self.text.trim().to_string(),
            full_string: self.full_string,
        }
    }
}

impl TextMerger {
    pub fn new(window_ms: u64) -> Self {
        Self {
            window_ms,
            pending: HashMap::new(),
            deadlines: BinaryHeap::new(),
            over_limit: HashSet::new(),
        }
    }

    /// 창 길이를 바꾼다. 원본 LunaHost도 `flushDelay`를 실행 중에 바꾼다.
    ///
    /// 새 창은 다음 `flush_expired`부터 적용된다. 이미 모인 조각은 그대로 두어야
    /// 창을 늘리는 순간 문장이 한가운데서 끊기지 않는다.
    pub fn set_window_ms(&mut self, window_ms: u64) {
        self.window_ms = window_ms;
        self.deadlines.clear();
        if window_ms > 0 {
            let window = Duration::from_millis(window_ms);
            self.deadlines.extend(
                self.pending
                    .iter()
                    .map(|(&source, pending)| (Reverse(pending.last_seen + window), source)),
            );
        }
    }

    #[cfg(test)]
    pub fn window_ms(&self) -> u64 {
        self.window_ms
    }

    /// 이벤트를 출처별 문장 버퍼에 적립한다.
    #[cfg(test)]
    pub fn submit(&mut self, event: HookText) -> Vec<HookText> {
        self.submit_at(event, Instant::now())
    }

    pub fn submit_at(&mut self, event: HookText, received_at: Instant) -> Vec<HookText> {
        let source = event.source;
        let window = Duration::from_millis(self.window_ms);
        let expired = self
            .pending
            .get(&source)
            .is_some_and(|pending| {
                self.window_ms > 0
                    && received_at
                        .checked_duration_since(pending.last_seen)
                        .is_some_and(|gap| gap >= window)
            })
            .then(|| {
                self.over_limit.remove(&source);
                self.pending.remove(&source)
            })
            .flatten()
            .map(|pending| pending.into_hook_text(source));
        let full_string = event.full_string;
        // 이어 붙일 버퍼를 뺀 나머지는 언제나 이번 이벤트의 값이다. 새 항목을
        // 빈 값으로 넣었다가 곧바로 덮어쓰지 않도록 두 경우를 나눠 적는다.
        let overlong = {
            let pending = match self.pending.entry(source) {
                Entry::Occupied(slot) => {
                    let pending = slot.into_mut();
                    pending.hook_name = event.hook_name;
                    pending.last_seen = received_at;
                    pending.full_string = full_string;
                    pending.char_count = pending
                        .char_count
                        .saturating_add(event.text.chars().count());
                    pending
                }
                Entry::Vacant(slot) => slot.insert(PendingText {
                    text: String::new(),
                    hook_name: event.hook_name,
                    last_seen: received_at,
                    full_string,
                    char_count: event.text.chars().count(),
                }),
            };
            // 원본 `TextThread::Push`처럼 온 것을 그대로 이어 붙인다. 같은 값과
            // 공백도 유효한 데이터다.
            pending.text.push_str(&event.text);
            // 원본은 한 번에 문장을 내는 훅(FULL_STRING)이 두 글자 이상을 냈을 때
            // 줄바꿈을 붙인다. 한 창에 모인 문장들이 서로 붙지 않게 한다.
            if full_string && event.text.encode_utf16().nth(1).is_some() {
                pending.text.push('\n');
            }
            pending.char_count > MAX_BUFFER_CHARS
        };
        if self.window_ms > 0 {
            self.deadlines.push((Reverse(received_at + window), source));
        }
        if overlong {
            self.over_limit.insert(source);
        }
        expired.into_iter().collect()
    }

    /// 마지막 조각 이후 window가 지난 출처를 완성된 문장으로 방출한다.
    ///
    /// 원본 `TextThread::Flush`와 같이 훅 종류를 가리지 않는다. 한 창 안에
    /// 도착한 FULL_STRING 문장들은 줄바꿈으로 이어져 한 번에 나간다. 창이
    /// 지나지 않아도 너무 길어진 출처는 함께 낸다.
    pub fn flush_expired(&mut self) -> Vec<HookText> {
        let now = Instant::now();
        let window = Duration::from_millis(self.window_ms);
        let mut expired = Vec::new();
        while let Some(&(Reverse(deadline), source)) = self.deadlines.peek() {
            if deadline > now {
                break;
            }
            self.deadlines.pop();
            let Some(pending) = self.pending.get(&source) else {
                continue;
            };
            // 같은 source가 갱신된 뒤 남은 오래된 heap 항목은 건너뛴다.
            if pending.last_seen + window != deadline {
                continue;
            }
            if let Some(pending) = self.pending.remove(&source) {
                self.over_limit.remove(&source);
                expired.push(pending.into_hook_text(source));
            }
        }
        // window == 0은 모든 버퍼가 즉시 만료되는 기존 의미를 유지한다.
        if self.window_ms == 0 {
            expired.extend(
                self.pending
                    .drain()
                    .map(|(source, pending)| pending.into_hook_text(source)),
            );
        }
        for source in self.over_limit.drain() {
            if let Some(pending) = self.pending.remove(&source) {
                expired.push(pending.into_hook_text(source));
            }
        }
        expired
    }

    /// 종료/attach 해제 시 전부 방출한다.
    pub fn drain_all(&mut self) -> Vec<HookText> {
        self.deadlines.clear();
        self.over_limit.clear();
        self.pending
            .drain()
            .map(|(source, pending)| pending.into_hook_text(source))
            .collect()
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/hook/text_bridge/merger.rs"]
mod tests;
