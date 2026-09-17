// minato_check.rs
// SSPなしで動くCLI構文チェッカー。
// ゴーストのホームディレクトリ（talks/main.mnt を含むディレクトリ）を渡すと、
// SSPに読み込ませずにparser::load_programとanalyzer::Analyzerを走らせ、
// 構文エラー・静的解析結果（未定義call、ループ外break等）を表示する。
// save.jsonの読み書きやSAORIのロードなど、実行系の副作用は発生させない。

use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use minato::analyzer::Analyzer;
use minato::parser::{load_program, LoadError, Talk};

fn main() -> ExitCode {
    let mut args = env::args();
    let _prog = args.next();

    let ghost_dir = match args.next() {
        Some(a) => PathBuf::from(a),
        None => {
            eprintln!("使い方: minato_check <ゴーストのホームディレクトリ>");
            return ExitCode::from(1);
        }
    };

    let main_mnt = ghost_dir.join("talks").join("main.mnt");

    let mut visited = HashSet::new();
    let (all_talks, all_funcs, _all_globals) = match load_program(&main_mnt, &mut visited) {
        Ok(r) => r,
        Err(LoadError::PreprocessError(msg)) => {
            eprintln!("[preprocess error] {}", msg);
            return ExitCode::from(1);
        }
        Err(LoadError::ParseError(msgs, path)) => {
            eprintln!("構文エラー ({}):", path.display());
            for m in &msgs {
                eprintln!("  {}", m);
            }
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

    let analyze_errors = Analyzer::new(talk_names, func_names).analyze(&talks, &all_funcs);

    if analyze_errors.is_empty() {
        println!("構文・静的解析ともに問題ありませんでした");
        return ExitCode::from(0);
    }

    let mut has_error = false;
    for e in &analyze_errors {
        println!("[{}] {}内 {}行目: {}", e.level, e.event, e.line, e.message);
        if e.level == "error" {
            has_error = true;
        }
    }

    if has_error {
        ExitCode::from(1)
    } else {
        ExitCode::from(0)
    }
}
