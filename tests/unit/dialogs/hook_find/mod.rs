use super::*;

/// 선택은 이 창의 thread_local과 프로세스 전역 스냅샷 양쪽에 기록된다.
/// thread_local은 테스트 스레드마다 갈리지만 전역은 하나뿐이라, 그것을
/// 건드리는 테스트끼리는 직렬화해야 서로의 값을 갈아 끼우지 않는다.
static SESSION_STATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_session_state() -> std::sync::MutexGuard<'static, ()> {
    SESSION_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[test]
fn text_received_with_no_dialog_starts_a_saved_view() {
    let _guard = lock_session_state();
    reset_session();
    observe_text(&HookText {
        source: HookSource {
            address: 0x1000,
            context: 0x2000,
            subcontext: 0x3000,
        },
        hook_name: "test".to_string(),
        text: "보존되어야 할 문장".to_string(),
        full_string: false,
    });

    SAVED_VIEW.with(|slot| {
        let view = slot.borrow();
        let view = view
            .as_ref()
            .expect("closed dialog must retain its stream view");
        assert_eq!(view.streams.len(), 1);
        assert_eq!(view.streams[0].history, "보존되어야 할 문장");
    });

    reset_session();
}

#[test]
fn only_selected_stream_is_accepted() {
    let selected = HookSource {
        address: 1,
        context: 2,
        subcontext: 3,
    };
    let other = HookSource {
        address: 4,
        context: 5,
        subcontext: 6,
    };

    SELECTED_SOURCE.with(|slot| slot.set(None));
    assert!(!accepts(selected));

    SELECTED_SOURCE.with(|slot| slot.set(Some(selected)));
    assert!(accepts(selected));
    assert!(!accepts(other));

    SELECTED_SOURCE.with(|slot| slot.set(None));
}

#[test]
fn saved_hook_name_selects_the_first_matching_stream() {
    let _guard = lock_session_state();
    reset_session();
    prepare_auto_select("SavedHook".into());
    let source = HookSource {
        address: 10,
        context: 20,
        subcontext: 30,
    };

    observe_text(&HookText {
        source,
        hook_name: "savedhook".into(),
        text: "자동 선택".into(),
        full_string: false,
    });

    assert!(accepts(source));
    reset_session();
}

/// 후킹 워커 스레드는 이 창의 thread_local을 볼 수 없다. 디버그 로그를 고른
/// 스레드로 좁히려면 선택이 전역 스냅샷까지 닿아야 하므로, 다른 스레드에서
/// 읽어 확인한다.
#[test]
fn the_selected_stream_is_visible_from_another_thread() {
    let _guard = lock_session_state();
    reset_session();
    let source = HookSource {
        address: 0x40,
        context: 0x41,
        subcontext: 0x42,
    };
    let other = HookSource {
        address: 0x50,
        context: 0x51,
        subcontext: 0x52,
    };

    prepare_auto_select("SavedHook".into());
    observe_text(&HookText {
        source,
        hook_name: "savedhook".into(),
        text: "고른 스레드".into(),
        full_string: false,
    });

    let seen = std::thread::spawn(move || {
        (
            crate::hook::is_selected_text_source(source),
            crate::hook::is_selected_text_source(other),
        )
    })
    .join()
    .expect("worker-thread view must not panic");
    assert_eq!(seen, (true, false));

    // 세션이 끝나면 다음 게임의 텍스트가 앞 선택으로 새어 나오면 안 된다.
    reset_session();
    let after_reset = std::thread::spawn(move || crate::hook::is_selected_text_source(source))
        .join()
        .expect("worker-thread view must not panic");
    assert!(!after_reset);
}
