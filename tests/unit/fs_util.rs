use super::*;

#[test]
fn atomic_write_replaces_existing_file_without_temp_residue() {
    let root = std::env::temp_dir().join(format!(
        "anemone-atomic-write-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).expect("create test directory");
    let path = root.join("config.toml");
    fs::write(&path, "old").expect("seed old file");

    atomic_write(&path, b"new contents").expect("atomic replace");

    assert_eq!(fs::read_to_string(&path).unwrap(), "new contents");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    fs::remove_dir_all(root).expect("remove test directory");
}

#[test]
fn atomic_write_retries_a_colliding_temp_name() {
    let root = std::env::temp_dir().join(format!(
        "anemone-atomic-collision-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let collision = root.join("collision.tmp");
    let fallback = root.join("fallback.tmp");
    fs::write(&collision, b"stale").unwrap();

    let (file, selected) = create_unique_temp_from([collision.clone(), fallback.clone()]).unwrap();
    drop(file);

    assert_eq!(selected, fallback);
    assert_eq!(fs::read(&collision).unwrap(), b"stale");
    fs::remove_dir_all(root).unwrap();
}
