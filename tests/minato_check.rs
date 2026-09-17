// minato_checkバイナリ（src/bin/minato_check.rs）の結合テスト。
// SSPを介さずに構文エラー・静的解析結果とexit codeが期待通りになることを確認する。

use std::fs;
use std::process::Command;

fn run(dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_minato_check"))
        .arg(dir)
        .output()
        .expect("minato_checkの実行に失敗しました")
}

fn write_main_mnt(dir: &std::path::Path, content: &str) {
    let talks_dir = dir.join("talks");
    fs::create_dir_all(&talks_dir).expect("talksディレクトリ作成失敗");
    fs::write(talks_dir.join("main.mnt"), content).expect("main.mnt書き込み失敗");
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
}

#[test]
fn test_undefined_call_reports_notice_and_exits_zero() {
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    write_main_mnt(
        dir.path(),
        "OnBoot => {\n    call 存在しない関数\n}\n",
    );

    let output = run(dir.path());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "noticeのみならexit codeは0であるべき: {:?}",
        output
    );
    assert!(
        stdout.contains("[notice]") && stdout.contains("2行目") && stdout.contains("存在しない関数"),
        "未定義callのnoticeが期待通り出ていない: {}",
        stdout
    );
}

#[test]
fn test_break_outside_loop_reports_error_and_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    write_main_mnt(
        dir.path(),
        "OnBoot => {\n    break\n}\n",
    );

    let output = run(dir.path());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(1),
        "ループ外breakはexit code 1であるべき: {:?}",
        output
    );
    assert!(
        stdout.contains("[error]") && stdout.contains("2行目"),
        "ループ外breakのerrorが期待通り出ていない: {}",
        stdout
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
