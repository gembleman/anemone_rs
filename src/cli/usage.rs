pub fn print_usage() {
    println!("anemone_rs — Windows 오버레이 번역 도구");
    println!();
    println!("USAGE:");
    println!("    anemone_rs.exe                              GUI 모드로 실행 (기본)");
    println!("    anemone_rs.exe <COMMAND> [ARGS...] [--json] 명령 실행");
    println!();
    println!("COMMANDS:");
    println!("    translate <TEXT> [--engine E] [--from L] [--to L] [--stdin]");
    println!(
        "                                  텍스트 번역. config.toml 의 키/엔진을 기본값으로 사용"
    );
    println!("    file-trans --in <FILE> --out <FILE> [--engine E] [--from L] [--to L]");
    println!("               [--format only|both|both-nl] [--no-trans-linefeed]");
    println!("                                  파일을 한 줄씩 번역해 출력 파일에 기록");
    println!("    list-engines                  지원하는 번역 엔진 출력");
    println!("    list-langs [--engine E]       엔진이 지원하는 언어 출력");
    println!("    config show                   현재 config.toml 내용 출력");
    println!(
        "    config get <KEY>              config 값을 점 경로로 조회 (예: translation.engine)"
    );
    println!("    config set <KEY> <VALUE>      config 값 변경 후 저장");
    println!("    config-path                   config.toml 의 절대 경로 출력");
    println!();
    println!("OPTIONS:");
    println!("    --json    결과를 JSON 한 줄로 출력 (기본은 사람이 읽기 좋은 plain text)");
    println!("    -h, --help  이 도움말 출력");
    println!();
    println!("ENGINE: eztrans | google | deepl | papago | llm");
    println!("LANG:   ISO 639-1 (예: ja, ko, en, zh)");
    println!();
    println!("CONFIG KEYS (대표):");
    println!("    translation.engine, translation.source_lang, translation.target_lang");
    println!("    translation.eztrans_dll_path, translation.eztrans_dat_path");
    println!("    translation.deepl_api_key, translation.papago_client_id,");
    println!("    translation.papago_client_secret");
    println!("    translation.llm.provider, translation.llm.model, translation.llm.api_key");
    println!(
        "    translation.llm.base_url, translation.llm.temperature, translation.llm.max_tokens"
    );
    println!("    clipboard_watch, click_through, magnetic_mode, background_visible,");
    println!("    border_visible, window_topmost, window_visible");
}
