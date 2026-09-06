"""rust-code-analysis-cli가 뽑은 JSON에서 인지 복잡도 초과 함수를 찾는다.

PowerShell 5.1의 ConvertFrom-Json은 대소문자만 다른 키('n1'과 'N1')를 중복으로
보고 거부하기 때문에, 이 집계만 Python으로 분리했다. lizard가 이미 Python을
요구하므로 새 의존성은 아니다.

사용법:
    python scripts/cognitive-complexity.py <json_dir> [--limit 20]

초과 함수를 한 줄에 하나씩 stdout으로 출력하고, 있으면 종료 코드 1로 끝난다.
"""

import argparse
import json
import os
import pathlib
import sys


def walk(node, source, found, limit):
    """중첩된 space 트리를 훑어 limit을 넘는 함수/클로저를 모은다."""
    cognitive = node.get("metrics", {}).get("cognitive", {}).get("sum")
    if node.get("kind") in ("function", "closure") and cognitive and cognitive > limit:
        found.append((cognitive, node.get("name"), source, node.get("start_line")))
    for child in node.get("spaces", []):
        walk(child, source, found, limit)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("json_dir", help="rust-code-analysis-cli -O json 출력 디렉터리")
    parser.add_argument("--limit", type=int, default=20, help="인지 복잡도 상한")
    args = parser.parse_args()

    found = []
    for path in pathlib.Path(args.json_dir).rglob("*.json"):
        with open(path, encoding="utf-8") as handle:
            document = json.load(handle)
        walk(document, document.get("name", str(path)), found, args.limit)

    found.sort(reverse=True)
    for cognitive, name, source, line in found:
        try:
            source = os.path.relpath(source)
        except ValueError:
            # 다른 드라이브에 있으면 절대 경로 그대로 둔다.
            pass
        print(f"{name} (인지 {cognitive}) {source}:{line}")

    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
