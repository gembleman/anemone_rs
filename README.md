# 아네모네_RS
2017년 프로그램 아네모네를 개선하고자 완전히 새롭게 만들었습니다.

### 다운로드  
최신버전 v0.1.1  
[extrans가 필요한 경우](https://github.com/gembleman/anemone_rs/releases/tag/starter_kit)  

[extrans가 필요 없는 경우](https://github.com/gembleman/anemone_rs/releases/tag/v0.1.1)

## 개선사항
1. 그래픽 프레임워크를 GDI에서 다이렉트X로 전환. GPU를 쓰는 덕분에 CPU 사용률 저하.
2. LLM API, 구글 등 번역 API 추가.
3. 캐시 기능 추가. 과거에 번역했던 문장을 다시 번역할 필요 없음.
4. 이지트랜스로 번역할 때, 파일 번역 성능 개선. 5배 정도 빨라짐.

## 커스텀 API 추가하기 (v0.1.1 기준)

내장 엔진(EzTrans, Google, DeepL, Papago, LLM) 외에 임의의 JSON REST API를 번역 엔진으로 등록할 수 있습니다.
Ollama, llama.cpp, LM Studio 같은 로컬 LLM 서버를 붙일 때 사용합니다.

설정은 **실행 파일과 같은 폴더의 `config.toml`** 을 직접 편집합니다.
프로그램이 켜져 있으면 종료한 뒤 편집하세요. 저장 시 덮어써집니다.

### 기본 형식

```toml
[translation]
engine = "custom"
source_lang = "ja"
target_lang = "ko"
custom_api = "Ollama"   # 아래 목록 중 사용할 이름. 비우면 첫 번째 항목

[[translation.custom_apis]]
name = "Ollama"
url = "http://localhost:11434/v1/chat/completions"
api_key = ""
auth_header = ""
auth_scheme = ""
headers = "{}"
request_template = '''{"model":"qwen2.5:7b","messages":[{"role":"system","content":"You are a translator. Translate the user text from {source} to {target}. Output only the translation."},{"role":"user","content":"{text}"}],"stream":false,"temperature":0.3}'''
response_path = "/choices/0/message/content"
```

`[[translation.custom_apis]]` 블록을 여러 개 두면 이름으로 전환할 수 있습니다.

### 설정 항목

| 항목 | 설명 |
|---|---|
| `name` | 프로그램에 표시될 이름. 비우거나 중복되면 안 됩니다. |
| `url` | POST 요청을 보낼 전체 주소. `http` 또는 `https`만 됩니다. |
| `api_key` | API 키. 로컬 서버는 보통 비워둡니다. |
| `auth_header` | 키를 실을 헤더 이름. 비우면 인증 헤더를 안 보냅니다. |
| `auth_scheme` | 키 앞에 붙일 값(예: `Bearer`). 비우면 키만 보냅니다. |
| `headers` | 추가로 보낼 헤더. JSON 객체 문자열이며 값은 전부 문자열이어야 합니다. |
| `request_template` | 보낼 JSON 본문. |
| `response_path` | 응답에서 번역문을 꺼낼 경로. |

`request_template`과 `headers` 안의 문자열에는 아래 자리표시자를 쓸 수 있습니다.

- `{text}` — 번역할 원문
- `{source}` — 원본 언어 코드 (예: `ja`)
- `{target}` — 번역할 언어 코드 (예: `ko`)
- `{api_key}` — 위에 적은 API 키

`response_path`는 `/choices/0/message/content` 같은 JSON Pointer 형식과
`choices.0.message.content` 같은 점 표기법을 모두 지원합니다.
비워두면 응답 본문 전체를 번역문으로 씁니다.

### Ollama

먼저 모델을 받아둡니다. `ollama pull qwen2.5:7b`

```toml
[[translation.custom_apis]]
name = "Ollama"
url = "http://localhost:11434/v1/chat/completions"
request_template = '''{"model":"qwen2.5:7b","messages":[{"role":"system","content":"You are a translator. Translate the user text from {source} to {target}. Output only the translation, no explanations."},{"role":"user","content":"{text}"}],"stream":false,"temperature":0.3}'''
response_path = "/choices/0/message/content"
```

`model`은 `ollama list`에 나오는 이름과 정확히 같아야 합니다.

### llama.cpp

`llama-server`를 먼저 띄웁니다. `llama-server -m model.gguf -c 4096 --port 8080`

```toml
[[translation.custom_apis]]
name = "llama.cpp"
url = "http://localhost:8080/v1/chat/completions"
request_template = '''{"messages":[{"role":"system","content":"You are a translator. Translate the user text from {source} to {target}. Output only the translation, no explanations."},{"role":"user","content":"{text}"}],"stream":false,"temperature":0.3,"n_predict":512}'''
response_path = "/choices/0/message/content"
```

llama-server는 모델을 하나만 올리므로 `model`을 적지 않아도 됩니다.

### LM Studio

LM Studio에서 모델을 불러온 뒤 Developer 탭에서 서버를 켭니다.

```toml
[[translation.custom_apis]]
name = "LM Studio"
url = "http://localhost:1234/v1/chat/completions"
request_template = '''{"model":"qwen2.5-7b-instruct","messages":[{"role":"system","content":"You are a translator. Translate the user text from {source} to {target}. Output only the translation, no explanations."},{"role":"user","content":"{text}"}],"stream":false,"temperature":0.3}'''
response_path = "/choices/0/message/content"
```

`model`에는 LM Studio에 표시된 모델 식별자를 넣습니다.

### 사용법

설정을 저장하고 프로그램을 켜면 설정 창의 번역 엔진에서 `Custom API`를 고를 수 있고,
그 아래 목록에서 등록한 이름을 선택합니다.

명령줄에서도 쓸 수 있습니다.

```
anemone_rs.exe translate --engine custom --source ja --target ko "テキスト"
anemone_rs.exe file-trans --engine custom -i input.txt -o output.txt
```

## 추후에 개선할 부분
1. 64비트로 전환.
2. 후킹 기능 추가.
3. 자체 번역 엔진 탑재.

## 주의
윈도우 10에서만 테스트했습니다. 그 이하 버전에서는 작동이 안 될 겁니다.

### 관련 문의나 버그 제보
오류나 버그가 생기면 이슈를 열거나,  
[개인 사이트](https://doujinkorea.com/)에 글 써주시면 됩니다.

## AI 사용 여부
초기 설계를 제외한 대부분의 코드는 AI가 생성했습니다.