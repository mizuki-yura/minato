// minato_check.rs
// SSPなしで動くCLI構文チェッカー。
// talks/main.mnt を直下に持つディレクトリ（ゴーストの ghost/master）を渡すと、
// SSPに読み込ませずにparser::load_programとanalyzer::Analyzerを走らせ、
// 構文エラー・静的解析結果（未定義call、ループ外break等）を表示する。
// save.jsonの読み書きやSAORIのロードなど、実行系の副作用は発生させない。
//
// 出力先: 診断はすべて標準エラー出力、問題なしのメッセージだけ標準出力。
// 終了コード: errorがある・パース/preprocessに失敗した → 1、notice/warningのみ・問題なし → 0。
//
// ビルド: cargo build --features cli --bin minato_check

use std::collections::{HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ariadne::{Color, Config, IndexType, Label, Report, ReportKind, Source};
use minato::analyzer::Analyzer;
use minato::diagnostic::{location, render_header, sort_diagnostics, Diagnostic, Level};
use minato::parser::{load_program, AssignOp, Expr, LoadError, PathSegment, Spanned, Stmt, Talk};

const USAGE: &str = "使い方: minato_check [--no-color] <ghost/masterのディレクトリ（talks/main.mnt を含むディレクトリ）>";

type LoadResult = Result<
    (
        Vec<Talk>,
        Vec<(String, Vec<String>, Vec<Spanned<Stmt>>)>,
        Vec<(Vec<PathSegment>, AssignOp, Expr)>,
    ),
    LoadError,
>;

/// chumskyの再帰下降パーサーは実サイズのmain.mntだと既定のスレッドスタック
/// （Windowsのメインスレッドは通常1MB程度）では足りずオーバーフローすることが
/// あるため、SSP向けDLL側のload_program_guardedと同様に大きいスタックを
/// 持つ別スレッドで実行する。
fn run_load_program(main_mnt: &Path) -> LoadResult {
    let main_mnt = main_mnt.to_path_buf();
    let spawned = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024) // 16MB
        .spawn(move || {
            let mut visited = HashSet::new();
            load_program(&main_mnt, &mut visited)
        });

    match spawned {
        Ok(handle) => match handle.join() {
            Ok(result) => result,
            Err(_) => Err(LoadError::PreprocessError(Diagnostic::error(
                "パース処理中に予期しないエラー（パニック）が発生しました",
            ))),
        },
        Err(e) => Err(LoadError::PreprocessError(Diagnostic::error(format!(
            "パース処理を開始できませんでした: {}",
            e
        )))),
    }
}

// ── 表示 ─────────────────────────────────────────────────

/// 診断のファイル名から本文を引くためのキャッシュ。
/// Diagnostic.fileはファイル名だけなので、talks配下から同じ名前のファイルを探して読み直す。
/// 見つからない・同名が複数ある・読めない場合はNone（抜粋なしで表示する）。
struct SourceFiles {
    talks_dir: PathBuf,
    cache: HashMap<String, Option<String>>,
}

impl SourceFiles {
    fn new(talks_dir: PathBuf) -> Self {
        Self { talks_dir, cache: HashMap::new() }
    }

    fn get(&mut self, file: &str) -> Option<&str> {
        if !self.cache.contains_key(file) {
            let mut found = Vec::new();
            find_files_named(&self.talks_dir, file, &mut found);
            let text = match found.as_slice() {
                [only] => std::fs::read_to_string(only).ok(),
                _ => None,
            };
            self.cache.insert(file.to_string(), text);
        }
        self.cache.get(file).and_then(|t| t.as_deref())
    }
}

fn find_files_named(dir: &Path, name: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_files_named(&path, name, out);
        } else if path.file_name().is_some_and(|n| n == name) {
            out.push(path);
        }
    }
}

/// (行, 桁)（どちらも1-indexed、桁は文字単位）を、本文全体の先頭からの文字オフセットにする。
/// ariadneはIndexType::Charで本文全体を文字単位で数えるので、\rも1文字として数える。
fn char_offset(text: &str, line: u32, col: u32) -> Option<usize> {
    let mut offset = 0usize;
    for (i, l) in text.split('\n').enumerate() {
        if i + 1 == line as usize {
            let len = l.trim_end_matches('\r').chars().count();
            let c = (col as usize).checked_sub(1)?;
            return if c <= len { Some(offset + c) } else { None };
        }
        offset += l.chars().count() + 1;
    }
    None
}

fn level_tag(level: Level, color: bool) -> String {
    let tag = format!("[{}]", level.as_str());
    if !color {
        return tag;
    }
    let code = match level {
        Level::Error => "31",
        Level::Warning => "33",
        Level::Notice => "36",
    };
    format!("\x1b[{}m{}\x1b[0m", code, tag)
}

/// 抜粋なしの1行表示。「[error] main.mntの4行目（OnBoot内）: メッセージ ヒント: …」
fn print_plain(d: &Diagnostic, color: bool) {
    eprintln!("{} {}", level_tag(d.level, color), render_header(d));
}

/// ソース抜粋付きの表示。col が無い・本文が読めない場合は print_plain にフォールバックする。
fn print_diagnostic(d: &Diagnostic, sources: &mut SourceFiles, color: bool) {
    let (Some(file), Some(line), Some(col)) = (d.file.as_deref(), d.line(), d.col()) else {
        return print_plain(d, color);
    };
    let Some(text) = sources.get(file) else {
        return print_plain(d, color);
    };
    let Some(start) = char_offset(text, line, col) else {
        return print_plain(d, color);
    };

    let kind = match d.level {
        Level::Error => ReportKind::Error,
        Level::Warning => ReportKind::Warning,
        Level::Notice => ReportKind::Custom("notice", Color::Cyan),
    };
    let title = match location(d) {
        Some(loc) => format!("{}: {}", loc, d.message),
        None => d.message.clone(),
    };
    let span = (file.to_string(), start..start + 1);
    let mut report = Report::build(kind, span.clone())
        .with_config(Config::default().with_color(color).with_index_type(IndexType::Char))
        .with_message(title)
        .with_label(Label::new(span).with_message("ここ").with_color(Color::Red));
    if let Some(h) = &d.hint {
        report = report.with_help(h);
    }
    let result = report
        .finish()
        .eprint((file.to_string(), Source::from(text.to_string())));
    if result.is_err() {
        print_plain(d, color);
    }
}

fn print_all(diags: &[Diagnostic], sources: &mut SourceFiles, color: bool) {
    for d in diags {
        print_diagnostic(d, sources, color);
    }
}

/// 標準エラー出力がコンソールなら、ANSIエスケープ（色）を解釈するVTモードを有効にする。
/// 旧来のコンソール（ドラッグ&ドロップで開くcmd等）は既定で無効なので、
/// 有効にできなかったときは false を返し、色なしで表示する。
/// コンソールでない（パイプ・リダイレクト先）ときは何もせず true。
fn enable_console_color() -> bool {
    use winapi::um::consoleapi::{GetConsoleMode, SetConsoleMode};
    use winapi::um::processenv::GetStdHandle;
    use winapi::um::winbase::STD_ERROR_HANDLE;
    use winapi::um::wincon::ENABLE_VIRTUAL_TERMINAL_PROCESSING;
    unsafe {
        let handle = GetStdHandle(STD_ERROR_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return true;
        }
        mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
            || SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

// ── main ─────────────────────────────────────────────────

fn main() -> ExitCode {
    let mut color = true;
    let mut ghost_dir: Option<PathBuf> = None;
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "--no-color" => color = false,
            "-h" | "--help" => {
                println!("{}", USAGE);
                return ExitCode::from(0);
            }
            a if a.starts_with("--") => {
                eprintln!("不明なオプションです: {}", a);
                eprintln!("{}", USAGE);
                return ExitCode::from(1);
            }
            a if ghost_dir.is_none() => ghost_dir = Some(PathBuf::from(a)),
            a => {
                eprintln!("ディレクトリは1つだけ指定してください: {}", a);
                eprintln!("{}", USAGE);
                return ExitCode::from(1);
            }
        }
    }
    if color {
        color = enable_console_color();
    }
    let Some(ghost_dir) = ghost_dir else {
        eprintln!("{}", USAGE);
        return ExitCode::from(1);
    };

    let talks_dir = ghost_dir.join("talks");
    let main_mnt = talks_dir.join("main.mnt");
    let mut sources = SourceFiles::new(talks_dir);

    let (all_talks, all_funcs, _all_globals) = match run_load_program(&main_mnt) {
        Ok(r) => r,
        Err(LoadError::PreprocessError(d)) => {
            print_all(&[d], &mut sources, color);
            return ExitCode::from(1);
        }
        Err(LoadError::ParseError(mut diags, _path)) => {
            sort_diagnostics(&mut diags);
            print_all(&diags, &mut sources, color);
            return ExitCode::from(1);
        }
    };

    // func_namesを先に作る（all_funcsをムーブする前に）
    let func_names: HashSet<String> = all_funcs
        .iter()
        .map(|(name, _, _)| name.clone())
        .collect();

    let mut talks: HashMap<String, Vec<Talk>> = HashMap::new();
    for talk in all_talks {
        talks.entry(talk.event.clone()).or_default().push(talk);
    }
    let talk_names: HashSet<String> = talks.keys().cloned().collect();

    let mut analyze_errors = Analyzer::new(talk_names, func_names).analyze(&talks, &all_funcs);

    if analyze_errors.is_empty() {
        println!("構文・静的解析ともに問題ありませんでした");
        return ExitCode::from(0);
    }

    sort_diagnostics(&mut analyze_errors);
    print_all(&analyze_errors, &mut sources, color);

    if analyze_errors.iter().any(|e| e.level == Level::Error) {
        ExitCode::from(1)
    } else {
        ExitCode::from(0)
    }
}

#[cfg(test)]
mod tests {
    use super::char_offset;

    #[test]
    fn char_offset_counts_chars() {
        let text = "湊: あ\n  let x\n";
        // 2行目3桁目は先頭行の4文字（湊・:・空白・あ）＋改行1文字＋2文字
        assert_eq!(char_offset(text, 2, 3), Some(7));
    }

    #[test]
    fn char_offset_counts_cr_as_char() {
        let text = "ab\r\ncd\r\n";
        assert_eq!(char_offset(text, 2, 1), Some(4));
    }

    #[test]
    fn char_offset_out_of_range() {
        assert_eq!(char_offset("ab\n", 1, 9), None);
        assert_eq!(char_offset("ab\n", 5, 1), None);
    }
}
