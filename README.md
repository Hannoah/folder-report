# folder-report

폴더를 스캔해서 **파일 현황 요약 리포트(Markdown)** 를 만드는 Rust 도구입니다.
대상 폴더의 파일은 **읽기만** 합니다.

## 리포트 내용
- 총 파일 수 / 총 용량
- 확장자별 파일 수·용량
- 용량 상위 N개 파일
- 최근 N일 내 수정된 파일

`.git`, `.venv`, `venv`, `__pycache__`, `node_modules`, 각종 cache 폴더는 제외됩니다.

## 빌드 / 사용법
```powershell
cargo install --path .          # 한 번만: 어느 폴더에서나 folder_report 로 실행 가능

folder_report <대상폴더> <저장경로>
```
```powershell
folder_report C:\Work D:\reports                 # 폴더에 저장 -> D:\reports\folder_report.md
folder_report C:\Work D:\reports\work.md         # 파일명 지정
folder_report C:\Work                            # 저장경로 생략 시 화면 출력
folder_report C:\Work work.md --top 20 --days 3  # 옵션(선택)
```
설치 없이 쓰려면 `cargo build --release` 후 `.\target\release\folder_report.exe`.

## 테스트
```powershell
cargo test
```

## 라이선스
[MIT](LICENSE)

## 동작 메모
- 심볼릭 링크/Windows 정션 폴더는 따라가지 않고(순환 방지), 링크 *파일*은 집계에서 제외합니다 (이중 집계 방지).
- 출력 파일(`-o`)이 스캔 폴더 안에 있어도 통계에서 제외됩니다.
- 권한 문제 등으로 읽지 못한 폴더/파일이 있으면 리포트 상단에 `⚠ 접근 불가로 건너뜀: N개`가 표시됩니다.
