use super::*;

#[test]
fn runtime_data_stays_next_to_executable() {
    let executable = Path::new(r"D:\Apps\Anemone");
    let paths = AppPaths::for_executable_dir(executable);

    assert_eq!(paths.data_dir(), executable);
    assert_eq!(paths.config_file(), executable.join("config.toml"));
    assert_eq!(paths.data_dir().join("logs"), executable.join("logs"));
}
