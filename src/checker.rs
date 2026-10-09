// checker.rs
// SSPなしで構文チェック・静的解析を行う共通処理。
// CLI（src/bin/minato_check.rs）とGUI（minato-checker-gui）の両方から呼び、
// 読み込み〜解析の流れとエラー文言を一か所にまとめる。
// save.jsonの読み書きやSAORIのロードなど、実行系の副作用は発生させない。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::analyzer::Analyzer;
use crate::diagnostic::{sort_diagnostics, Diagnostic, Level};
use crate::parser::{load_program, AssignOp, Expr, LoadError, PathSegment, Spanned, Stmt, Talk};

/// チェック結果。diagnosticsはファイル名・行番号順に並べ済み（読み込み前の失敗は1件のみ）。
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub diagnostics: Vec<Diagnostic>,
    /// 読み込み・パースの段階で失敗した（静的解析まで進めなかった）。
    pub load_failed: bool,
}

impl CheckResult {
    /// 読み込みに失敗したか、errorが1件でもあるか（CLIの終了コード1に相当）。
    pub fn has_error(&self) -> bool {
        self.load_failed || self.diagnostics.iter().any(|d| d.level == Level::Error)
    }
}

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

/// talks/main.mnt を直下に持つディレクトリ（ゴーストの ghost/master）をチェックする。
pub fn check_ghost(master_dir: &Path) -> CheckResult {
    let main_mnt = master_dir.join("talks").join("main.mnt");

    let (all_talks, all_funcs, _all_globals) = match run_load_program(&main_mnt) {
        Ok(r) => r,
        Err(LoadError::PreprocessError(d)) => return CheckResult { diagnostics: vec![d], load_failed: true },
        Err(LoadError::ParseError(mut diags, _path)) => {
            sort_diagnostics(&mut diags);
            return CheckResult { diagnostics: diags, load_failed: true };
        }
    };

    let func_names: HashSet<String> = all_funcs
        .iter()
        .map(|(name, _, _)| name.clone())
        .collect();

    let mut talks: HashMap<String, Vec<Talk>> = HashMap::new();
    for talk in all_talks {
        talks.entry(talk.event.clone()).or_default().push(talk);
    }
    let talk_names: HashSet<String> = talks.keys().cloned().collect();

    let mut diagnostics = Analyzer::new(talk_names, func_names).analyze(&talks, &all_funcs);
    sort_diagnostics(&mut diagnostics);
    CheckResult { diagnostics, load_failed: false }
}

/// ドロップ・選択されたパスから、talks/main.mnt を持つディレクトリを探す。
/// - ghost/master そのもの → そのまま
/// - ゴーストのルート（ghost/master を含む） → ghost/master
/// - .mnt ファイルやtalks配下のフォルダ → 親をさかのぼって見つかったところ
pub fn resolve_master_dir(path: &Path) -> Option<PathBuf> {
    let has_main = |d: &Path| d.join("talks").join("main.mnt").is_file();
    let start = if path.is_file() { path.parent()? } else { path };
    let nested = start.join("ghost").join("master");
    if has_main(&nested) {
        return Some(nested);
    }
    start.ancestors().find(|d| has_main(d)).map(Path::to_path_buf)
}

/// 診断のファイル名から本文を引くためのキャッシュ。
/// Diagnostic.fileはファイル名だけなので、talks配下から同じ名前のファイルを探して読み直す。
/// 見つからない・同名が複数ある・読めない場合はNone（抜粋なしで表示する）。
pub struct SourceFiles {
    talks_dir: PathBuf,
    cache: HashMap<String, Option<String>>,
}

impl SourceFiles {
    pub fn new(talks_dir: PathBuf) -> Self {
        Self { talks_dir, cache: HashMap::new() }
    }

    pub fn get(&mut self, file: &str) -> Option<&str> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ghost() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let talks = dir.path().join("ghost").join("master").join("talks");
        std::fs::create_dir_all(talks.join("sub")).unwrap();
        std::fs::write(talks.join("main.mnt"), "OnBoot => {\n    湊: おはよう\n}\n").unwrap();
        std::fs::write(talks.join("sub").join("a.mnt"), "").unwrap();
        dir
    }

    #[test]
    fn resolve_from_root_master_and_file() {
        let dir = ghost();
        let master = dir.path().join("ghost").join("master");
        assert_eq!(resolve_master_dir(dir.path()), Some(master.clone()));
        assert_eq!(resolve_master_dir(&master), Some(master.clone()));
        let file = master.join("talks").join("sub").join("a.mnt");
        assert_eq!(resolve_master_dir(&file), Some(master));
    }

    #[test]
    fn resolve_none_without_main() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve_master_dir(dir.path()), None);
    }

    #[test]
    fn check_ok_and_error() {
        let dir = ghost();
        let master = dir.path().join("ghost").join("master");
        assert!(check_ghost(&master).diagnostics.is_empty());
        std::fs::write(master.join("talks").join("main.mnt"), "OnBoot => {\n    break\n}\n").unwrap();
        assert!(check_ghost(&master).has_error());
    }
}
