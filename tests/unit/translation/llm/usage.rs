use super::*;

struct FixedDate(Date);

impl DateProvider for FixedDate {
    fn today(&self) -> Date {
        self.0
    }
}

fn date(year: i32, month: time::Month, day: u8) -> Date {
    Date::from_calendar_date(year, month, day).unwrap()
}

#[test]
fn injected_local_date_changes_the_bucket_at_midnight() {
    let mut file = UsageFile::default();
    apply_record(
        &mut file,
        &FixedDate(date(2026, time::Month::July, 18)),
        10,
        20,
    );
    apply_record(
        &mut file,
        &FixedDate(date(2026, time::Month::July, 19)),
        30,
        40,
    );

    assert_eq!(file.days["2026-07-18"].calls, 1);
    assert_eq!(file.days["2026-07-19"].calls, 1);
    assert_eq!(file.days["2026-07-19"].input_bytes, 30);
}

#[test]
fn retention_keeps_the_newest_thirty_injected_dates() {
    let mut file = UsageFile::default();
    for day in 1u8..=31 {
        apply_record(
            &mut file,
            &FixedDate(date(2026, time::Month::January, day)),
            1,
            1,
        );
    }

    assert_eq!(file.days.len(), RETAIN_DAYS);
    assert!(!file.days.contains_key("2026-01-01"));
    assert!(file.days.contains_key("2026-01-31"));
}

#[test]
fn usage_file_is_replaced_atomically_with_valid_json() {
    let root = std::env::temp_dir().join(format!(
        "anemone-usage-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = root.join("llm_usage.json");
    let mut file = UsageFile::default();
    file.days.insert(
        "2026-07-18".to_string(),
        DayStats {
            calls: 3,
            input_bytes: 20,
            output_bytes: 40,
        },
    );

    save_to_path(&file, &path).unwrap();

    let loaded: UsageFile = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(loaded.days["2026-07-18"].calls, 3);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}
