// minato_check.rs
// SSPなしで動くCLI構文チェッカー。
// talks/main.mnt を直下に持つディレクトリ（ゴーストの ghost/master）を渡すと、
// SSPに読み込ませずにchecker::check_ghost（パーサーと静的解析）を走らせ、
// 構文エラー・静的解析結果（未定義call、ループ外break等）を表示する。
// save.jsonの読み書きやSAORIのロードなど、実行系の副作用は発生させない。
//
// 出力先: 診断はすべて標準エラー出力、問題なしのメッセージだけ標準出力。
// 終了コード: errorがある・パース/preprocessに失敗した → 1、notice/warningのみ・問題なし → 0。
//
// ビルド: cargo build --features cli --bin minato_check

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use ariadne::{Color, Config, IndexType, Label, Report, ReportKind, Source};
use minato::checker::{check_ghost, SourceFiles};
use minato::diagnostic::{location, render_header, Diagnostic, Level};

const USAGE: &str = "使い方: minato_check [--no-color] <ghost/masterのディレクトリ（talks/main.mnt を含むディレクトリ）>";

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

    let mut sources = SourceFiles::new(ghost_dir.join("talks"));
    let result = check_ghost(&ghost_dir);

    if result.diagnostics.is_empty() {
        println!("構文・静的解析ともに問題ありませんでした");
        return ExitCode::from(0);
    }

    print_all(&result.diagnostics, &mut sources, color);

    if result.has_error() {
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
