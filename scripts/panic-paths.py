"""프로덕션 Rust 코드에 남은 패닉 유발 지점을 찾는다.

`#[cfg(test)]`가 붙은 아이템(모듈이든 함수든)은 테스트 전용이므로 제외한다.
속성이 붙은 아이템의 끝은 중괄호 균형으로 판단하고, 세미콜론으로 끝나는
선언(`#[cfg(test)] mod tests;`)은 그 줄에서 끝난 것으로 본다.

사용법:
    python scripts/panic-paths.py [--path src]

찾은 지점을 한 줄에 하나씩 stdout으로 출력하고, 있으면 종료 코드 1로 끝난다.
"""

import argparse
import pathlib
import re
import sys

PANIC = re.compile(r"\.unwrap\(\)|\.expect\(|unreachable!|panic!\(|todo!|unimplemented!")


def test_only_ranges(lines):
    """`#[cfg(test)]`가 붙은 아이템이 차지하는 줄 번호 집합을 만든다."""
    excluded = set()
    index = 0
    while index < len(lines):
        if lines[index].strip() != "#[cfg(test)]":
            index += 1
            continue

        # 속성이 여러 개 이어질 수 있다: #[cfg(test)] #[path = "..."] mod tests;
        cursor = index + 1
        while cursor < len(lines) and lines[cursor].lstrip().startswith("#["):
            cursor += 1
        if cursor >= len(lines):
            break

        excluded.update(range(index, cursor + 1))

        # 본문이 없는 선언은 그 줄에서 끝난다.
        if lines[cursor].rstrip().endswith(";"):
            index = cursor + 1
            continue

        # 본문이 있으면 중괄호가 닫힐 때까지가 이 아이템의 범위다.
        depth = lines[cursor].count("{") - lines[cursor].count("}")
        while depth > 0 and cursor + 1 < len(lines):
            cursor += 1
            excluded.add(cursor)
            depth += lines[cursor].count("{") - lines[cursor].count("}")
        index = cursor + 1
    return excluded


def scan(path):
    lines = path.read_text(encoding="utf-8").split("\n")
    excluded = test_only_ranges(lines)
    hits = []
    for number, line in enumerate(lines):
        if number in excluded:
            continue
        # 주석은 실제 패닉 경로가 아니다.
        if line.lstrip().startswith("//"):
            continue
        if PANIC.search(line):
            hits.append((number + 1, line.strip()))
    return hits


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--path", default="src", help="검사할 디렉터리")
    args = parser.parse_args()

    total = 0
    for source in sorted(pathlib.Path(args.path).rglob("*.rs")):
        for number, text in scan(source):
            print(f"{source}:{number}: {text}")
            total += 1
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main())
