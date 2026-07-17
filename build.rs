use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let mut res = winresource::WindowsResource::new();
    res.set_manifest_file("app.manifest");
    res.compile().expect("Failed to compile Windows resources");

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
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            mirror_dir(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            copy_if_newer(&src_path, &dst_path)?;
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
