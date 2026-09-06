use super::*;

#[test]
fn module_matching_uses_full_case_insensitive_path() {
    assert!(module_paths_match(
        Path::new(r"C:\Games\Anemone\lunahook.dll"),
        Path::new(r"c:/games/anemone/LUNAHOOK.DLL"),
    ));
    assert!(!module_paths_match(
        Path::new(r"C:\Games\Anemone\lunahook.dll"),
        Path::new(r"C:\Mods\lunahook.dll"),
    ));
}

#[test]
fn module_matching_falls_back_when_canonicalize_fails() {
    assert!(module_paths_match(
        Path::new(r"\\?\C:\Games\lunahook.dll"),
        Path::new(r"c:/games/LUNAHOOK.DLL"),
    ));
}

#[test]
fn normalized_path_converts_the_unc_extended_prefix() {
    assert_eq!(
        normalized_windows_path(Path::new(r"\\?\UNC\server\share\file.dll")),
        r"\\server\share\file.dll"
    );
}

#[test]
fn normalized_path_lowercases_and_unifies_separators() {
    assert_eq!(
        normalized_windows_path(Path::new(r"C:/Games/Anemone/LunaHook.DLL")),
        r"c:\games\anemone\lunahook.dll"
    );
}
