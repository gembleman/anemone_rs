use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let mut res = winresource::WindowsResource::new();
    res.add_toolkit_include(true);
    res.set_manifest_file("app.manifest");
    res.set_icon("assets/Anemone.ico");
    let settings_rc = fs::read_to_string("resources/settings.rc")
        .expect("Failed to read settings dialog resource");
    res.append_rc_content(&settings_rc);
    let hook_settings_rc = fs::read_to_string("resources/hook_settings.rc")
        .expect("Failed to read hook settings dialog resource");
    res.append_rc_content(&hook_settings_rc);
    let glossary_rc = fs::read_to_string("resources/glossary.rc")
        .expect("Failed to read glossary dialog resource");
    res.append_rc_content(&glossary_rc);
    let file_trans_progress_rc = fs::read_to_string("resources/file_trans_progress.rc")
        .expect("Failed to read file translation progress dialog resource");
    res.append_rc_content(&file_trans_progress_rc);
    let file_trans_rc = fs::read_to_string("resources/file_trans.rc")
        .expect("Failed to read file translation dialog resource");
    res.append_rc_content(&file_trans_rc);
    let translate_rc = fs::read_to_string("resources/translate.rc")
        .expect("Failed to read translation dialog resource");
    res.append_rc_content(&translate_rc);
    let backlog_rc =
        fs::read_to_string("resources/backlog.rc").expect("Failed to read backlog dialog resource");
    res.append_rc_content(&backlog_rc);
    res.compile().expect("Failed to compile Windows resources");

    println!("cargo:rerun-if-changed=assets/Anemone.ico");
    println!("cargo:rerun-if-changed=resources/settings.rc");
    println!("cargo:rerun-if-changed=resources/hook_settings.rc");
    println!("cargo:rerun-if-changed=resources/glossary.rc");
    println!("cargo:rerun-if-changed=resources/file_trans_progress.rc");
    println!("cargo:rerun-if-changed=resources/file_trans.rc");
    println!("cargo:rerun-if-changed=resources/translate.rc");
    println!("cargo:rerun-if-changed=resources/backlog.rc");

    copy_eztrans_dll();
}

/// 프로젝트 루트의 `eztrans_dll/` 폴더를 실행 파일과 같은 위치(target/<triple>/<profile>/)로
/// 미러링한다. 변경된(혹은 새로 생긴) 파일만 복사한다.
fn copy_eztrans_dll() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let src = manifest_dir.join("eztrans_dll");

    println!("cargo:rerun-if-changed=eztrans_dll");

    if !src.exists() {
        println!(
            "cargo:warning=eztrans_dll 폴더가 없습니다: {} (복사를 건너뜁니다)",
            src.display()
        );
        return;
    }

    let Some(target_dir) = resolve_target_dir() else {
        println!(
            "cargo:warning=target 디렉터리 추정에 실패했습니다. eztrans_dll 복사를 건너뜁니다."
        );
        return;
    };
    let dst = target_dir.join("eztrans_dll");

    if let Err(e) = mirror_dir(&src, &dst) {
        println!("cargo:warning=eztrans_dll 복사 실패: {e}");
    }
}

/// OUT_DIR(`target/<triple>/<profile>/build/<pkg-hash>/out`)에서 `target/<triple>/<profile>/`를
/// 거슬러 올라간다.
fn resolve_target_dir() -> Option<PathBuf> {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR")?);
    // out_dir: .../target/<triple>?/<profile>/build/<pkg-hash>/out
    // 부모 3단계 위로 올라가면 <profile> 디렉터리.
    let profile_dir = out_dir.ancestors().nth(3)?.to_path_buf();
    Some(profile_dir)
}

fn mirror_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    let mut source_names = HashSet::new();
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        source_names.insert(entry.file_name());
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            mirror_dir(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            copy_if_newer(&src_path, &dst_path)?;
        }
    }
    for entry in fs::read_dir(dst)? {
        let entry = entry?;
        if source_names.contains(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn copy_if_newer(src: &Path, dst: &Path) -> std::io::Result<()> {
    let needs_copy = match (fs::metadata(src), fs::metadata(dst)) {
        (Ok(s), Ok(d)) => match (s.modified(), d.modified()) {
            (Ok(sm), Ok(dm)) => sm > dm || s.len() != d.len(),
            _ => true,
        },
        (Ok(_), Err(_)) => true,
        (Err(e), _) => return Err(e),
    };

    if needs_copy {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
    }
    Ok(())
}
