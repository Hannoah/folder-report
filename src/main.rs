//! folder_report - 폴더를 스캔해 파일 현황 요약 리포트(Markdown)를 생성합니다.
//! 읽기 전용 도구입니다. 스캔 대상 폴더의 파일은 수정/삭제하지 않습니다.
//!
//! 사용법: folder_report [대상폴더] [저장경로]   (인자 없이 실행하면 질문형)

use chrono::{DateTime, Duration, Local};
use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const DEFAULT_EXCLUDES: &[&str] = &[
    ".git", ".venv", "venv", "__pycache__", "node_modules",
    ".pytest_cache", ".ruff_cache", ".mypy_cache", "target",
];

const SHORT_USAGE: &str = "사용법: folder_report [대상폴더] [저장경로]   (자세한 도움말: folder_report --help)";

fn help_text() -> String {
    format!(
        "folder_report - 폴더 파일 현황을 Markdown 리포트로 만듭니다 (읽기 전용)\n\n\
         사용법:\n  \
         folder_report                        질문에 답하는 방식으로 실행\n  \
         folder_report <대상폴더>             리포트를 화면에 출력\n  \
         folder_report <대상폴더> <저장경로>  리포트를 파일로 저장\n\n\
         저장경로가 기존 폴더면 그 안에 folder_report.md 로 저장합니다.\n\n\
         옵션:\n  \
         --top N           상위 표에 보여 줄 개수 (기본 10)\n  \
         --days N          '최근 수정' 기준 일수 (기본 7)\n  \
         --exclude 이름    제외할 폴더 이름 추가 (여러 번 사용 가능)\n  \
         --all             기본 제외 폴더까지 포함해 전부 스캔\n  \
         -o, --output 경로 저장경로를 옵션으로 지정\n  \
         -h, --help        이 도움말\n\n\
         기본 제외 폴더: {}",
        DEFAULT_EXCLUDES.join(" ")
    )
}

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

/// 스캔 결과
struct Scan {
    files: Vec<FileInfo>,
    /// 접근 실패로 건너뛴 항목의 경로
    skipped: Vec<PathBuf>,
    /// 제외 규칙에 걸려 내용을 보지 않은 폴더
    excluded: Vec<PathBuf>,
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

fn pct(part: u64, total: u64) -> String {
    if total == 0 { "0.0%".to_string() } else { format!("{:.1}%", part as f64 * 100.0 / total as f64) }
}

fn num(n: usize) -> String {
    commas(&n.to_string())
}

// ponytail: 심볼릭 링크/정션은 따라가지 않음 (file_type()은 링크를 따라가지 않고,
// Windows 정션도 is_symlink()로 잡힘). 링크 파일은 집계에서 제외됨.
fn scan(root: &Path, excludes: &[String], progress: bool) -> Scan {
    let mut sc = Scan { files: Vec::new(), skipped: Vec::new(), excluded: Vec::new() };
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            sc.skipped.push(dir);
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else { sc.skipped.push(dir.clone()); continue };
            let Ok(ft) = entry.file_type() else { sc.skipped.push(entry.path()); continue };
            if ft.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if excludes.iter().any(|x| x.eq_ignore_ascii_case(&name)) {
                    sc.excluded.push(entry.path());
                } else {
                    stack.push(entry.path());
                }
            } else if ft.is_file() {
                match entry.metadata().and_then(|md| Ok((md.len(), md.modified()?))) {
                    Ok((size, mtime)) => {
                        sc.files.push(FileInfo { path: entry.path(), size, modified: mtime.into() });
                        if progress && sc.files.len() % 2000 == 0 {
                            eprint!("\r스캔 중... {}개", num(sc.files.len()));
                        }
                    }
                    Err(_) => sc.skipped.push(entry.path()),
                }
            }
        }
    }
    if progress {
        eprint!("\r{}\r", " ".repeat(40)); // 진행 표시 지우기
    }
    sc
}

/// (이름 -> (파일 수, 용량)) 를 용량 큰 순으로 정렬
fn sorted_by_size(map: HashMap<String, (u64, u64)>) -> Vec<(String, u64, u64)> {
    let mut v: Vec<_> = map.into_iter().map(|(k, (c, s))| (k, c, s)).collect();
    v.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    v
}

fn build_report(root: &Path, sc: &Scan, top: usize, days: i64, now: DateTime<Local>) -> String {
    let files = &sc.files;
    let total: u64 = files.iter().map(|f| f.size).sum();
    let rel_path = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");

    // ---- 요약 ----
    let mut out = format!(
        "# 폴더 리포트: `{}`\n\n- 생성 시각: {}\n- 총 파일 수: {}개\n- 총 용량: {}\n",
        display_path(root),
        now.format("%Y-%m-%d %H:%M:%S"),
        num(files.len()),
        human_size(total),
    );
    if let (Some(min), Some(max)) = (
        files.iter().map(|f| f.modified).min(),
        files.iter().map(|f| f.modified).max(),
    ) {
        out += &format!("- 수정일 범위: {} ~ {}\n", min.format("%Y-%m-%d"), max.format("%Y-%m-%d"));
    }
    if !sc.skipped.is_empty() {
        out += &format!("- ⚠ 접근 불가로 건너뜀: {}개 (실제보다 적게 집계됨)\n", num(sc.skipped.len()));
    }

    // ---- 확장자별 ----
    let mut by_ext: HashMap<String, (u64, u64)> = HashMap::new();
    for f in files {
        let e = by_ext.entry(f.ext()).or_default();
        e.0 += 1;
        e.1 += f.size;
    }
    out += "\n## 확장자별 현황\n\n| 확장자 | 파일 수 | 용량 | 비율 |\n|---|---:|---:|---:|\n";
    for (ext, cnt, size) in sorted_by_size(by_ext) {
        out += &format!("| {ext} | {} | {} | {} |\n", num(cnt as usize), human_size(size), pct(size, total));
    }

    // ---- 하위 폴더별 (최상위 1단계) ----
    let mut by_dir: HashMap<String, (u64, u64)> = HashMap::new();
    for f in files {
        let r = f.path.strip_prefix(root).unwrap_or(&f.path);
        let mut c = r.components();
        let key = match (c.next(), c.next()) {
            (Some(first), Some(_)) => format!("{}/", first.as_os_str().to_string_lossy()),
            _ => "(루트 직하 파일)".to_string(),
        };
        let e = by_dir.entry(key).or_default();
        e.0 += 1;
        e.1 += f.size;
    }
    let dirs = sorted_by_size(by_dir);
    out += &format!("\n## 하위 폴더별 현황 (용량 상위 {top}개)\n\n| 폴더 | 파일 수 | 용량 | 비율 |\n|---|---:|---:|---:|\n");
    for (name, cnt, size) in dirs.iter().take(top) {
        out += &format!("| {name} | {} | {} | {} |\n", num(*cnt as usize), human_size(*size), pct(*size, total));
    }
    if dirs.len() > top {
        out += &format!("\n_외 {}개 항목 생략_\n", dirs.len() - top);
    }

    // ---- 수정 시점별 분포 ----
    const LABELS: [&str; 5] = ["7일 이내", "8~30일", "31~90일", "91일~1년", "1년 초과"];
    const LIMITS: [i64; 4] = [7, 30, 90, 365];
    let mut buckets = [(0u64, 0u64); 5];
    for f in files {
        let age = now - f.modified;
        let i = LIMITS.iter().position(|&d| age <= Duration::days(d)).unwrap_or(4);
        buckets[i].0 += 1;
        buckets[i].1 += f.size;
    }
    out += "\n## 수정 시점별 분포\n\n| 마지막 수정 | 파일 수 | 용량 | 비율 |\n|---|---:|---:|---:|\n";
    for (label, (cnt, size)) in LABELS.iter().zip(buckets) {
        out += &format!("| {label} | {} | {} | {} |\n", num(cnt as usize), human_size(size), pct(size, total));
    }

    // ---- 용량 상위 파일 ----
    out += &format!("\n## 용량 상위 {top}개 파일\n\n| 파일 | 용량 | 수정일 |\n|---|---:|---|\n");
    let mut by_size: Vec<&FileInfo> = files.iter().collect();
    by_size.sort_by_key(|f| Reverse(f.size));
    for f in by_size.iter().take(top) {
        out += &format!("| {} | {} | {} |\n", rel_path(&f.path), human_size(f.size), f.modified.format("%Y-%m-%d"));
    }

    // ---- 최근 수정 ----
    let cutoff = now - Duration::days(days);
    let mut recent: Vec<&FileInfo> = files.iter().filter(|f| f.modified >= cutoff).collect();
    recent.sort_by_key(|f| Reverse(f.modified));
    out += &format!("\n## 최근 {days}일 내 수정된 파일 ({}개)\n\n", num(recent.len()));
    if recent.is_empty() {
        out += "_해당 없음_\n";
    } else {
        out += "| 파일 | 수정 시각 |\n|---|---|\n";
        for f in recent.iter().take(top) {
            out += &format!("| {} | {} |\n", rel_path(&f.path), f.modified.format("%Y-%m-%d %H:%M"));
        }
        if recent.len() > top {
            out += &format!("\n_외 {}개 생략_\n", recent.len() - top);
        }
    }

    // ---- 제외된 폴더 ----
    if !sc.excluded.is_empty() {
        // 소문자 이름 -> (표시 이름, 개수, 위치 예시)
        let mut groups: HashMap<String, (String, usize, String)> = HashMap::new();
        for p in &sc.excluded {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let g = groups.entry(name.to_lowercase()).or_insert((name, 0, rel_path(p)));
            g.1 += 1;
        }
        let mut v: Vec<_> = groups.into_values().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out += "\n## 제외된 폴더 (내용은 집계하지 않음)\n\n| 폴더 이름 | 개수 | 위치 예시 |\n|---|---:|---|\n";
        for (name, cnt, example) in v {
            out += &format!("| {name} | {} | {example} |\n", num(cnt));
        }
        out += "\n_포함해서 집계하려면 `--all` 옵션을 사용하세요._\n";
    }

    // ---- 접근하지 못한 항목 ----
    if !sc.skipped.is_empty() {
        out += "\n## 접근하지 못한 항목\n\n";
        for p in sc.skipped.iter().take(10) {
            out += &format!("- `{}`\n", rel_path(p));
        }
        if sc.skipped.len() > 10 {
            out += &format!("\n_외 {}개 생략_\n", sc.skipped.len() - 10);
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

/// 공백, 보이지 않는 BOM, 드래그 앤 드롭 시 붙는 따옴표 제거
fn clean_path(s: &str) -> String {
    let junk = |c: char| c.is_whitespace() || c == '\u{feff}' || c == '"' || c == '\'';
    s.trim_matches(junk).to_string()
}

type Failure = (String, u8); // (메시지, 종료 코드)

fn usage_err(msg: &str) -> Failure {
    (format!("{msg}\n{SHORT_USAGE}"), 2)
}

struct Opts {
    folder: Option<String>,
    output: Option<String>,
    top: usize,
    days: i64,
    all: bool,
    extra_excludes: Vec<String>,
}

/// Ok(None) = 도움말 요청
fn parse(args: &[String]) -> Result<Option<Opts>, Failure> {
    let mut o = Opts { folder: None, output: None, top: 10, days: 7, all: false, extra_excludes: vec![] };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(None),
            "--all" => o.all = true,
            "-o" | "--output" | "--top" | "--days" | "--exclude" => {
                let v = it.next().ok_or_else(|| usage_err(&format!("{a} 값이 필요합니다")))?;
                let bad = || usage_err(&format!("{a}는 정수여야 합니다"));
                match a.as_str() {
                    "--top" => o.top = v.parse().map_err(|_| bad())?,
                    "--days" => o.days = v.parse().map_err(|_| bad())?,
                    "--exclude" => o.extra_excludes.push(v.clone()),
                    _ => o.output = Some(v.clone()),
                }
            }
            _ if a.starts_with('-') => return Err(usage_err(&format!("알 수 없는 옵션: {a}"))),
            // 위치 인자: 첫 번째는 대상 폴더, 두 번째는 저장 경로
            _ if o.folder.is_none() => o.folder = Some(a.clone()),
            _ if o.output.is_none() => o.output = Some(a.clone()),
            _ => return Err(usage_err("인자가 너무 많습니다")),
        }
    }
    if o.all && !o.extra_excludes.is_empty() {
        return Err(usage_err("--all 과 --exclude 는 함께 쓸 수 없습니다"));
    }
    Ok(Some(o))
}

/// 프롬프트는 stderr로 출력해서 리포트(stdout)와 섞이지 않게 함
fn ask(prompt: &str) -> String {
    eprint!("{prompt}");
    let _ = io::stderr().flush();
    let mut s = String::new();
    let _ = io::stdin().read_line(&mut s);
    clean_path(&s)
}

fn run(args: &[String]) -> Result<(), Failure> {
    let mut opts = match parse(args)? {
        Some(o) => o,
        None => {
            println!("{}", help_text());
            return Ok(());
        }
    };

    // 인자가 하나도 없으면 질문형으로 진행
    if args.is_empty() {
        eprintln!("folder_report - 폴더 현황 리포트 만들기 (도움말: folder_report --help)\n");
        let folder = ask("대상 폴더 경로 (폴더를 끌어다 놓아도 됩니다): ");
        if folder.is_empty() {
            return Err(usage_err("대상 폴더를 입력하지 않았습니다"));
        }
        let output = ask("저장 경로 (폴더 또는 파일명, 비우면 화면에 출력): ");
        opts.folder = Some(folder);
        opts.output = (!output.is_empty()).then_some(output);
    }

    let folder = opts.folder.ok_or_else(|| usage_err("대상 폴더를 지정해 주세요"))?;
    // 저장 경로가 기존 폴더면 그 안에 기본 파일명으로 저장
    let output = opts.output.map(|o| {
        let p = Path::new(&o);
        if p.is_dir() { p.join("folder_report.md").to_string_lossy().into_owned() } else { o }
    });
    let root = fs::canonicalize(&folder)
        .ok()
        .filter(|p| p.is_dir())
        .ok_or_else(|| (format!("오류: 폴더를 찾을 수 없습니다: {folder}\n(경로에 공백이 있으면 따옴표로 감싸 주세요)"), 1))?;

    let excludes: Vec<String> = if opts.all {
        vec![]
    } else {
        DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).chain(opts.extra_excludes).collect()
    };

    // 출력 파일이 스캔 폴더 안에 있어도 이전 리포트가 통계에 섞이지 않게 제외
    let out_path = output.as_ref().and_then(|o| fs::canonicalize(o).ok());
    let mut sc = scan(&root, &excludes, io::stderr().is_terminal());
    sc.files.retain(|f| Some(&f.path) != out_path.as_ref());

    let report = build_report(&root, &sc, opts.top, opts.days, Local::now());
    match output {
        Some(o) => {
            fs::write(&o, report).map_err(|e| (format!("오류: 저장 실패: {e}"), 1))?;
            let abs = fs::canonicalize(&o).unwrap_or_else(|_| PathBuf::from(&o));
            let total: u64 = sc.files.iter().map(|f| f.size).sum();
            println!("리포트 저장 완료: {}", display_path(&abs));
            print!("  파일 {}개 · {}", num(sc.files.len()), human_size(total));
            if !sc.skipped.is_empty() {
                print!(" · ⚠ 접근 불가 {}개", num(sc.skipped.len()));
            }
            println!();
        }
        None => print!("{report}"),
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = run(&args);
    if let Err((msg, _)) = &result {
        eprintln!("{msg}");
    }
    // 인자 없이(더블클릭 등) 실행한 경우 창이 바로 닫히지 않게 대기
    if args.is_empty() {
        eprint!("\nEnter 키를 누르면 종료합니다...");
        let _ = io::stderr().flush();
        let _ = io::stdin().read_line(&mut String::new());
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err((_, code)) => ExitCode::from(code),
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
        let old = SystemTime::now() - StdDuration::from_secs(40 * 86400);
        fs::File::options().write(true).open(tmp.join("noext")).unwrap().set_modified(old).unwrap();
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn defaults() -> Vec<String> {
        DEFAULT_EXCLUDES.iter().map(|x| x.to_string()).collect()
    }

    fn report_of(root: &Path) -> String {
        build_report(root, &scan(root, &defaults(), false), 5, 7, Local::now())
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
        let mut names: Vec<_> = scan(t.path(), &defaults(), false)
            .files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["a.txt", "b.csv", "noext"]);
    }

    #[test]
    fn scan_custom_and_all_excludes() {
        let t = tempfile::tempdir().unwrap();
        make_tree(t.path());
        // 사용자 지정 제외 (대소문자 무시)
        let mut ex = defaults();
        ex.push("SUB".into());
        assert_eq!(scan(t.path(), &ex, false).files.len(), 2);
        // 전부 포함
        assert_eq!(scan(t.path(), &[], false).files.len(), 4);
    }

    #[test]
    fn report_content() {
        let t = tempfile::tempdir().unwrap();
        make_tree(t.path());
        let r = report_of(t.path());
        assert!(r.contains("총 파일 수: 3개"));
        assert!(r.contains("| .csv | 1 | 2.0 KB |"));
        assert!(r.contains("(확장자 없음)"));
        assert!(r.contains("최근 7일 내 수정된 파일 (2개)")); // noext는 40일 전
    }

    #[test]
    fn report_has_new_sections() {
        let t = tempfile::tempdir().unwrap();
        make_tree(t.path());
        let r = report_of(t.path());
        // 하위 폴더별 + 루트 직하 파일
        assert!(r.contains("| sub/ | 1 | 2.0 KB |"));
        assert!(r.contains("| (루트 직하 파일) | 2 |"));
        // 수정 시점별 분포: 7일 이내 2개, 31~90일 1개 (noext)
        assert!(r.contains("| 7일 이내 | 2 |"));
        assert!(r.contains("| 31~90일 | 1 |"));
        assert!(r.contains("| 1년 초과 | 0 |"));
        // 제외된 폴더
        assert!(r.contains("## 제외된 폴더"));
        assert!(r.contains("| __pycache__ | 1 | __pycache__ |"));
        // 수정일 범위, 비율
        assert!(r.contains("- 수정일 범위:"));
        assert!(r.contains('%'));
    }

    #[test]
    fn report_without_excluded_has_no_section() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("a.txt"), "x").unwrap();
        assert!(!report_of(t.path()).contains("## 제외된 폴더"));
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
    fn all_flag_includes_excluded_dirs() {
        let t = tempfile::tempdir().unwrap();
        make_tree(t.path());
        let out = t.path().join("r.md");
        run(&s(&[t.path().to_str().unwrap(), out.to_str().unwrap(), "--all"])).unwrap();
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.contains("총 파일 수: 4개"));
        assert!(!text.contains("## 제외된 폴더"));
    }

    #[test]
    fn parse_errors() {
        assert_eq!(parse(&s(&["x", "--all", "--exclude", "a"])).err().unwrap().1, 2);
        assert_eq!(parse(&s(&["--exclude"])).err().unwrap().1, 2);
        assert_eq!(parse(&s(&["--top", "abc"])).err().unwrap().1, 2);
        assert!(parse(&s(&["--help"])).unwrap().is_none());
    }

    #[test]
    fn clean_path_strips_quotes_and_spaces() {
        assert_eq!(clean_path("  \"C:\\My Work\"  \r\n"), "C:\\My Work");
        assert_eq!(clean_path("'D:\\x'"), "D:\\x");
        assert_eq!(clean_path("\n"), "");
        assert_eq!(clean_path("\u{feff}\"D:\\x\"\r\n"), "D:\\x"); // 붙여넣기/파이프 시 BOM
    }

    #[test]
    fn display_path_strips_extended_prefix() {
        assert_eq!(display_path(Path::new("\\\\?\\C:\\x")), "C:\\x");
        assert_eq!(display_path(Path::new("\\\\?\\UNC\\srv\\share\\d")), "\\\\srv\\share\\d");
        assert_eq!(display_path(Path::new("C:\\plain")), "C:\\plain");
    }

    #[test]
    fn unreadable_dir_is_counted_and_listed() {
        let t = tempfile::tempdir().unwrap();
        let sc = scan(&t.path().join("nope"), &defaults(), false); // read_dir 실패 = 접근 불가
        assert!(sc.files.is_empty());
        assert_eq!(sc.skipped.len(), 1);
        let r = build_report(t.path(), &sc, 5, 7, Local::now());
        assert!(r.contains("접근 불가로 건너뜀: 1개"));
        assert!(r.contains("## 접근하지 못한 항목"));
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
        assert_eq!(scan(t.path(), &defaults(), false).files.len(), 3);
    }
}
