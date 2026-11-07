//! folder_report - 폴더를 스캔해 파일 현황 요약 리포트(Markdown)를 생성합니다.
//! 읽기 전용 도구입니다. 스캔 대상 폴더의 파일은 수정/삭제하지 않습니다.
//!
//! 사용법: folder_report <폴더경로> [-o report.md] [--top 10] [--days 7]

use chrono::{DateTime, Duration, Local};
use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const EXCLUDE_DIRS: &[&str] = &[
    ".git", ".venv", "venv", "__pycache__", "node_modules",
    ".pytest_cache", ".ruff_cache", ".mypy_cache",
];

const USAGE: &str = "사용법: folder_report <대상폴더> [저장경로] [--top 10] [--days 7]\n\
                     (저장경로 생략 시 화면 출력, 폴더를 주면 그 안에 folder_report.md로 저장. -o 저장경로 도 가능)";

struct FileInfo {
    path: PathBuf,
    size: u64,
    modified: DateTime<Local>,
}

impl FileInfo {
    fn ext(&self) -> String {
        match self.path.extension() {
            Some(e) => format!(".{}", e.to_string_lossy().to_lowercase()),
            None => "(확장자 없음)".to_string(),
        }
    }
}

/// "1234567.5" -> "1,234,567.5"
fn commas(s: &str) -> String {
    let (int, frac) = s.split_once('.').map_or((s, None), |(a, b)| (a, Some(b)));
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    match frac {
        Some(f) => format!("{out}.{f}"),
        None => out,
    }
}

fn human_size(bytes: u64) -> String {
    let mut n = bytes as f64;
    for unit in ["B", "KB", "MB"] {
        if n < 1024.0 {
            return if unit == "B" {
                format!("{} B", commas(&format!("{n:.0}")))
            } else {
                format!("{} {unit}", commas(&format!("{n:.1}")))
            };
        }
        n /= 1024.0;
    }
    format!("{} GB", commas(&format!("{n:.1}")))
}

// ponytail: 심볼릭 링크/정션은 따라가지 않음 (file_type()은 링크를 따라가지 않고,
// Windows 정션도 is_symlink()로 잡힘). 링크 파일은 집계에서 제외됨.
/// (파일 목록, 접근 실패로 건너뛴 항목 수)
fn scan(root: &Path) -> (Vec<FileInfo>, usize) {
    let mut files = Vec::new();
    let mut skipped = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            skipped += 1;
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else { skipped += 1; continue };
            let Ok(ft) = entry.file_type() else { skipped += 1; continue };
            if ft.is_dir() {
                if !EXCLUDE_DIRS.contains(&entry.file_name().to_string_lossy().as_ref()) {
                    stack.push(entry.path());
                }
            } else if ft.is_file() {
                match entry.metadata().and_then(|md| Ok((md.len(), md.modified()?))) {
                    Ok((size, mtime)) => files.push(FileInfo { path: entry.path(), size, modified: mtime.into() }),
                    Err(_) => skipped += 1,
                }
            }
        }
    }
    (files, skipped)
}

fn build_report(root: &Path, files: &[FileInfo], top: usize, days: i64, now: DateTime<Local>, skipped: usize) -> String {
    let total: u64 = files.iter().map(|f| f.size).sum();

    let mut by_ext: HashMap<String, (u64, u64)> = HashMap::new();
    for f in files {
        let e = by_ext.entry(f.ext()).or_default();
        e.0 += 1;
        e.1 += f.size;
    }
    let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");

    let mut out = format!(
        "# 폴더 리포트: `{}`\n\n- 생성 시각: {}\n- 총 파일 수: {}개\n- 총 용량: {}\n{}\n\
         ## 확장자별 현황\n\n| 확장자 | 파일 수 | 용량 |\n|---|---:|---:|\n",
        display_path(root),
        now.format("%Y-%m-%d %H:%M:%S"),
        commas(&files.len().to_string()),
        human_size(total),
        if skipped > 0 {
            format!("- ⚠ 접근 불가로 건너뜀: {}개 (실제보다 적게 집계됨)\n", commas(&skipped.to_string()))
        } else {
            String::new()
        },
    );
    let mut exts: Vec<_> = by_ext.into_iter().collect();
    exts.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then_with(|| a.0.cmp(&b.0)));
    for (ext, (cnt, size)) in exts {
        out += &format!("| {ext} | {} | {} |\n", commas(&cnt.to_string()), human_size(size));
    }

    out += &format!("\n## 용량 상위 {top}개 파일\n\n| 파일 | 용량 | 수정일 |\n|---|---:|---|\n");
    let mut by_size: Vec<&FileInfo> = files.iter().collect();
    by_size.sort_by_key(|f| Reverse(f.size));
    for f in by_size.iter().take(top) {
        out += &format!("| {} | {} | {} |\n", rel(&f.path), human_size(f.size), f.modified.format("%Y-%m-%d"));
    }

    let cutoff = now - Duration::days(days);
    let mut recent: Vec<&FileInfo> = files.iter().filter(|f| f.modified >= cutoff).collect();
    recent.sort_by_key(|f| Reverse(f.modified));
    out += &format!("\n## 최근 {days}일 내 수정된 파일 ({}개)\n\n", recent.len());
    if recent.is_empty() {
        out += "_해당 없음_\n";
    } else {
        out += "| 파일 | 수정 시각 |\n|---|---|\n";
        for f in recent.iter().take(top) {
            out += &format!("| {} | {} |\n", rel(&f.path), f.modified.format("%Y-%m-%d %H:%M"));
        }
        if recent.len() > top {
            out += &format!("\n_외 {}개 생략_\n", recent.len() - top);
        }
    }
    out
}

/// Windows 확장 경로 접두사를 표시용으로 제거: \\?\C:\x -> C:\x, \\?\UNC\srv\share -> \\srv\share
fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else {
        s.strip_prefix("\\\\?\\").unwrap_or(&s).to_string()
    }
}

type Failure = (String, u8); // (메시지, 종료 코드)

fn usage_err(msg: &str) -> Failure {
    (format!("{msg}\n{USAGE}"), 2)
}

fn run(args: &[String]) -> Result<(), Failure> {
    let (mut folder, mut output, mut top, mut days) = (None, None, 10usize, 7i64);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "-o" | "--output" | "--top" | "--days" => {
                let v = it.next().ok_or_else(|| usage_err(&format!("{a} 값이 필요합니다")))?;
                let bad = || usage_err(&format!("{a}는 정수여야 합니다"));
                match a.as_str() {
                    "--top" => top = v.parse().map_err(|_| bad())?,
                    "--days" => days = v.parse().map_err(|_| bad())?,
                    _ => output = Some(v.clone()),
                }
            }
            _ if a.starts_with('-') => return Err(usage_err(&format!("알 수 없는 옵션: {a}"))),
            // 위치 인자: 첫 번째는 대상 폴더, 두 번째는 저장 경로
            _ if folder.is_none() => folder = Some(a.clone()),
            _ if output.is_none() => output = Some(a.clone()),
            _ => return Err(usage_err("인자가 너무 많습니다")),
        }
    }
    let folder = folder.ok_or_else(|| (USAGE.to_string(), 2))?;
    // 저장 경로가 기존 폴더면 그 안에 기본 파일명으로 저장
    let output = output.map(|o| {
        let p = Path::new(&o);
        if p.is_dir() { p.join("folder_report.md").to_string_lossy().into_owned() } else { o }
    });
    let root = fs::canonicalize(&folder)
        .ok()
        .filter(|p| p.is_dir())
        .ok_or_else(|| (format!("오류: 폴더를 찾을 수 없습니다: {folder}"), 1))?;

    // 출력 파일이 스캔 폴더 안에 있어도 이전 리포트가 통계에 섞이지 않게 제외
    let out_path = output.as_ref().and_then(|o| fs::canonicalize(o).ok());
    let (mut files, skipped) = scan(&root);
    files.retain(|f| Some(&f.path) != out_path.as_ref());

    let report = build_report(&root, &files, top, days, Local::now(), skipped);
    match output {
        Some(o) => {
            fs::write(&o, report).map_err(|e| (format!("오류: 저장 실패: {e}"), 1))?;
            let abs = fs::canonicalize(&o).unwrap_or_else(|_| PathBuf::from(&o));
            println!("리포트 저장 완료: {}", display_path(&abs));
        }
        None => print!("{report}"),
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err((msg, code)) => {
            eprintln!("{msg}");
            ExitCode::from(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration as StdDuration, SystemTime};

    fn make_tree(tmp: &Path) {
        fs::write(tmp.join("a.txt"), "hello").unwrap(); // 5 B
        fs::create_dir(tmp.join("sub")).unwrap();
        fs::write(tmp.join("sub/b.csv"), vec![b'x'; 2048]).unwrap(); // 2 KB
        fs::write(tmp.join("noext"), "12").unwrap();
        fs::create_dir(tmp.join("__pycache__")).unwrap();
        fs::write(tmp.join("__pycache__/skip.pyc"), "zz").unwrap(); // 제외 대상
        let old = SystemTime::now() - StdDuration::from_secs(30 * 86400);
        fs::File::options().write(true).open(tmp.join("noext")).unwrap().set_modified(old).unwrap();
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(500), "500 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_size(2048 * 1024 * 1024 * 1024), "2,048.0 GB");
    }

    #[test]
    fn scan_excludes_cache() {
        let t = tempfile::tempdir().unwrap();
        make_tree(t.path());
        let mut names: Vec<_> = scan(t.path()).0
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["a.txt", "b.csv", "noext"]);
    }

    #[test]
    fn report_content() {
        let t = tempfile::tempdir().unwrap();
        make_tree(t.path());
        let r = build_report(t.path(), &scan(t.path()).0, 5, 7, Local::now(), 0);
        assert!(r.contains("총 파일 수: 3개"));
        assert!(r.contains("| .csv | 1 | 2.0 KB |"));
        assert!(r.contains("(확장자 없음)"));
        assert!(r.contains("최근 7일 내 수정된 파일 (2개)")); // noext는 30일 전
    }

    #[test]
    fn main_writes_file_and_excludes_itself() {
        let t = tempfile::tempdir().unwrap();
        make_tree(t.path());
        let out = t.path().join("report.md");
        let args = s(&[t.path().to_str().unwrap(), "-o", out.to_str().unwrap()]);
        run(&args).unwrap();
        run(&args).unwrap(); // 두 번째 실행에도 자기 자신이 안 섞여야 함
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.starts_with("# 폴더 리포트"));
        assert!(text.contains("총 파일 수: 3개"));
    }

    #[test]
    fn display_path_strips_extended_prefix() {
        assert_eq!(display_path(Path::new("\\\\?\\C:\\x")), "C:\\x");
        assert_eq!(display_path(Path::new("\\\\?\\UNC\\srv\\share\\d")), "\\\\srv\\share\\d");
        assert_eq!(display_path(Path::new("C:\\plain")), "C:\\plain");
    }

    #[test]
    fn unreadable_dir_is_counted_as_skipped() {
        let t = tempfile::tempdir().unwrap();
        let (files, skipped) = scan(&t.path().join("nope")); // read_dir 실패 = 접근 불가
        assert!(files.is_empty());
        assert_eq!(skipped, 1);
        let r = build_report(t.path(), &files, 5, 7, Local::now(), skipped);
        assert!(r.contains("접근 불가로 건너뜀: 1개"));
    }

    #[test]
    fn second_positional_is_output_and_dir_gets_default_name() {
        let t = tempfile::tempdir().unwrap();
        make_tree(t.path());
        let src = t.path().join("src_dir");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("x.txt"), "x").unwrap();
        let out_dir = t.path().join("out");
        fs::create_dir(&out_dir).unwrap();

        run(&s(&[src.to_str().unwrap(), out_dir.to_str().unwrap()])).unwrap(); // 폴더 → 기본 파일명
        assert!(out_dir.join("folder_report.md").exists());

        let named = t.path().join("my.md");
        run(&s(&[src.to_str().unwrap(), named.to_str().unwrap()])).unwrap(); // 파일 경로
        assert!(named.exists());

        assert_eq!(run(&s(&["a", "b", "c"])).unwrap_err().1, 2); // 인자 과다
    }

    #[test]
    fn missing_folder_is_error_1() {
        let t = tempfile::tempdir().unwrap();
        let nope = t.path().join("nope");
        assert_eq!(run(&s(&[nope.to_str().unwrap()])).unwrap_err().1, 1);
    }

    #[cfg(windows)]
    #[test]
    fn scan_skips_junction_loop() {
        let t = tempfile::tempdir().unwrap();
        make_tree(t.path());
        let ok = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(t.path().join("sub").join("loop")) // mklink는 '/' 구분자를 거부함
            .arg(t.path())
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok);
        assert_eq!(scan(t.path()).0.len(), 3);
    }
}
