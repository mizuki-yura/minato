// minato_checkバイナリ（src/bin/minato_check.rs）の結合テスト。
// SSPを介さずに構文エラー・静的解析結果とexit codeが期待通りになることを確認する。
// 実行: cargo test --features cli

use std::fs;
use std::process::Command;

fn run_with(dir: &std::path::Path, opts: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_minato_check"))
        .args(opts)
        .arg(dir)
        .output()
        .expect("minato_checkの実行に失敗しました")
}

fn run(dir: &std::path::Path) -> std::process::Output {
    run_with(dir, &["--no-color"])
}

fn write_mnt(dir: &std::path::Path, name: &str, content: &str) {
    let talks_dir = dir.join("talks");
    fs::create_dir_all(&talks_dir).expect("talksディレクトリ作成失敗");
    fs::write(talks_dir.join(name), content).expect("mnt書き込み失敗");
}

fn write_main_mnt(dir: &std::path::Path, content: &str) {
    write_mnt(dir, "main.mnt", content);
}

#[test]
fn test_no_problems_exits_zero() {
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    write_main_mnt(
        dir.path(),
        "OnBoot => {\n    湊: おはよう\n}\n",
    );

    let output = run(dir.path());
    assert!(output.status.success(), "exit codeが0であるべき: {:?}", output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("問題ありませんでした"),
        "問題なしメッセージが出ていない: {}",
        stdout
    );
    assert!(output.stderr.is_empty(), "問題なしなのに標準エラーに出力がある: {:?}", output);
}

#[test]
fn test_undefined_call_reports_notice_and_exits_zero() {
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    write_main_mnt(
        dir.path(),
        "OnBoot => {\n    call 存在しない関数\n}\n",
    );

    let output = run(dir.path());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "noticeのみならexit codeは0であるべき: {:?}",
        output
    );
    assert!(
        stderr.contains("[notice]") && stderr.contains("2行目") && stderr.contains("存在しない関数"),
        "未定義callのnoticeが期待通り出ていない: {}",
        stderr
    );
    assert!(output.stdout.is_empty(), "診断が標準出力に出ている: {:?}", output);
}

#[test]
fn test_break_outside_loop_reports_error_and_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    write_main_mnt(
        dir.path(),
        "OnBoot => {\n    break\n}\n",
    );

    let output = run(dir.path());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "ループ外breakはexit code 1であるべき: {:?}",
        output
    );
    assert_eq!(
        stderr.trim_end(),
        "[error] main.mntの2行目（OnBoot内）: ループの外で break を使っています",
        "ループ外breakのerrorが期待通り出ていない"
    );
}

#[test]
fn test_parse_error_reports_to_stderr_and_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    // 「=」を欠いたletは構文エラーになる
    write_main_mnt(
        dir.path(),
        "OnBoot => {\n    let x\n}\n",
    );

    let output = run(dir.path());
    assert_eq!(
        output.status.code(),
        Some(1),
        "構文エラーはexit code 1であるべき: {:?}",
        output
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.is_empty(),
        "構文エラーが標準エラー出力に出ていない: {:?}",
        output
    );
}

#[test]
fn test_parse_error_shows_source_excerpt() {
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    write_main_mnt(
        dir.path(),
        "OnBoot => {\n    湊: おはよう\n    let x\n}\n",
    );

    let output = run(dir.path());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("main.mntの3行目"), "位置が出ていない: {}", stderr);
    // ariadneの抜粋: ファイル名:行:桁（桁は文字単位）と該当行の本文
    assert!(stderr.contains("main.mnt:3:5"), "抜粋の位置が文字単位でない: {}", stderr);
    assert!(stderr.contains("let x"), "ソース抜粋が出ていない: {}", stderr);
    assert!(!stderr.contains('\x1b'), "--no-colorなのに色が付いている: {:?}", stderr);
}

#[test]
fn test_parse_error_in_included_file_shows_its_excerpt() {
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    write_main_mnt(
        dir.path(),
        "include \"sub/sub.mnt\"\nOnBoot => {\n    湊: おはよう\n}\n",
    );
    fs::create_dir_all(dir.path().join("talks").join("sub")).unwrap();
    write_mnt(dir.path(), "sub/sub.mnt", "OnClose => {\n    let y\n}\n");

    let output = run(dir.path());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr.contains("sub.mnt:2:5") && stderr.contains("let y"), "include先の抜粋が出ていない: {}", stderr);
}

#[test]
fn test_preprocess_error_without_col_is_plain_line() {
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    write_main_mnt(dir.path(), "OnBoot => {\n    湊: おはよう\n");

    let output = run(dir.path());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr.trim_end(),
        "[error] main.mntの1行目: 1行目の「{」が閉じられていません ヒント: 対応する「}」を書いてください"
    );
}

#[test]
fn test_color_is_on_by_default() {
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    write_main_mnt(dir.path(), "OnBoot => {\n    break\n}\n");

    let output = run_with(dir.path(), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains('\x1b'), "既定で色が付いていない: {:?}", stderr);
}

#[test]
fn test_missing_main_mnt_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    // talks/main.mntを作らない

    let output = run(dir.path());
    assert_eq!(
        output.status.code(),
        Some(1),
        "main.mntが無い場合はexit code 1であるべき: {:?}",
        output
    );
}

#[test]
fn test_no_argument_shows_usage_and_exits_one() {
    let output = Command::new(env!("CARGO_BIN_EXE_minato_check"))
        .output()
        .expect("minato_checkの実行に失敗しました");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ghost/master") && stderr.contains("talks/main.mnt"), "{}", stderr);
}
