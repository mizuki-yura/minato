// main.rs
// 湊（minato）のGUI構文チェッカー。
// ゴーストのフォルダか .mnt ファイルをドロップ（またはダイアログで選択）すると、
// CLIのminato_checkと同じ minato::checker::check_ghost を走らせて結果を一覧表示する。
// エラー文言はminato本体のDiagnosticをそのまま使い、ここでは組み立てない。

// Windowsでダブルクリック起動したときにコンソール窓を出さない（他のOSでは無視される）
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText};
use minato::checker::{check_ghost, resolve_master_dir, CheckResult};
use minato::diagnostic::{Diagnostic, Level};

const GUIDE: &str = "ゴーストのフォルダ（または .mnt ファイル）を、この窓にドラッグ＆ドロップしてください。\n\
ドロップできないときは「フォルダを選ぶ」「ファイルを選ぶ」ボタンから選べます。\n\
書いたトークに書き間違いがないかを、SSPを起動せずに確かめられます。";

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("湊 構文チェッカー")
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([480.0, 360.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "minato-checker-gui",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

// ── 日本語フォント ───────────────────────────────────────

/// OSに入っている日本語フォントの候補（パス, .ttc内の番号）。上から順に試す。
/// 再配布しないためライセンス表記が増えず、exeも大きくならない。
fn font_candidates() -> Vec<(PathBuf, u32)> {
    let mut v = Vec::new();
    if cfg!(windows) {
        let fonts = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts");
        v.push((fonts.join("YuGothM.ttc"), 0)); // 游ゴシック Medium
        v.push((fonts.join("meiryo.ttc"), 0)); // メイリオ
        v.push((fonts.join("msgothic.ttc"), 0)); // MS ゴシック
    }
    if cfg!(target_os = "macos") {
        v.push((PathBuf::from("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc"), 0));
    }
    v
}

/// 見つかった日本語フォントを設定する。見つからなければ false。
/// 本文は英数字も日本語フォントで描く（既定フォントと混ぜると英数字の高さがずれるため）。
fn install_japanese_font(ctx: &egui::Context) -> bool {
    let Some((bytes, index)) = font_candidates()
        .into_iter()
        .find_map(|(path, index)| std::fs::read(path).ok().map(|b| (b, index)))
    else {
        return false;
    };
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "japanese".to_owned(),
        Arc::new(FontData { index, ..FontData::from_owned(bytes) }),
    );
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "japanese".to_owned());
    fonts.families.entry(FontFamily::Monospace).or_default().push("japanese".to_owned());
    ctx.set_fonts(fonts);
    true
}

// ── 状態 ─────────────────────────────────────────────────

enum Outcome {
    /// まだ何も選ばれていない
    Empty,
    /// 選ばれたが、まだチェックしていない
    Ready,
    /// talks/main.mnt が見つからなかった
    NotFound,
    Checked { master: PathBuf, result: CheckResult },
}

/// 画面の状態。前回開いたフォルダの記憶などを足すときはここに持たせる。
struct App {
    font_ok: bool,
    target: Option<PathBuf>,
    outcome: Outcome,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let font_ok = install_japanese_font(&cc.egui_ctx);
        let mut app = Self { font_ok, target: None, outcome: Outcome::Empty };
        // exeのアイコンにフォルダをドロップして起動したときは、それをすぐチェックする
        if let Some(arg) = std::env::args_os().nth(1) {
            app.select(PathBuf::from(arg), true);
        }
        app
    }

    fn select(&mut self, path: PathBuf, run_now: bool) {
        self.target = Some(path);
        self.outcome = Outcome::Ready;
        if run_now {
            self.run_check();
        }
    }

    fn run_check(&mut self) {
        let Some(target) = &self.target else { return };
        self.outcome = match resolve_master_dir(target) {
            Some(master) => {
                let result = check_ghost(&master);
                Outcome::Checked { master, result }
            }
            None => Outcome::NotFound,
        };
    }
}

fn level_style(level: Level) -> (&'static str, Color32) {
    match level {
        Level::Error => ("エラー", Color32::from_rgb(0xd0, 0x30, 0x30)),
        Level::Warning => ("警告", Color32::from_rgb(0xc0, 0x80, 0x00)),
        Level::Notice => ("お知らせ", Color32::from_rgb(0x30, 0x70, 0xc0)),
    }
}

/// 診断1件の表示。周辺コードの表示を足すときは、この行をクリック可能にして
/// minato::checker::SourceFiles から本文を引く。
fn diagnostic_row(ui: &mut egui::Ui, d: &Diagnostic) {
    let (label, color) = level_style(d.level);
    egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(1.5, color))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).strong().color(color));
                ui.label(RichText::new(d.file.as_deref().unwrap_or("（ファイル不明）")).strong());
                if let Some(line) = d.line() {
                    ui.label(format!("{}行目", line));
                }
                if let Some(ctx) = &d.context {
                    ui.weak(format!("（{}内）", ctx));
                }
            });
            ui.label(&d.message);
            if let Some(h) = &d.hint {
                ui.label(RichText::new(format!("ヒント: {}", h)).italics());
            }
        });
}

fn count(result: &CheckResult, level: Level) -> usize {
    result.diagnostics.iter().filter(|d| d.level == level).count()
}

fn display_path(p: &Path) -> String {
    p.display().to_string()
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ドロップされたら、そのまま1回チェックする
        let dropped: Option<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));
        if let Some(path) = dropped {
            self.select(path, true);
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            ui.add_space(6.0);
            if !self.font_ok {
                ui.colored_label(Color32::RED, "Japanese font not found. Text may not be displayed correctly.");
            }
            ui.label(GUIDE);
            ui.add_space(6.0);

            // ドロップ領域
            let stroke_color = if hovering { ui.visuals().selection.stroke.color } else { ui.visuals().weak_text_color() };
            egui::Frame::group(ui.style())
                .stroke(egui::Stroke::new(if hovering { 3.0 } else { 1.5 }, stroke_color))
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.vertical_centered(|ui| {
                        let text = match (&self.target, hovering) {
                            (_, true) => "ここで離してください".to_owned(),
                            (Some(t), false) => format!("対象: {}", display_path(t)),
                            (None, false) => "ここにフォルダか .mnt ファイルをドロップ".to_owned(),
                        };
                        ui.label(RichText::new(text).size(16.0));
                    });
                });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("フォルダを選ぶ").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.select(p, true);
                    }
                }
                if ui.button("ファイルを選ぶ").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("湊スクリプト", &["mnt"]).pick_file() {
                        self.select(p, true);
                    }
                }
                ui.separator();
                let label = if matches!(self.outcome, Outcome::Checked { .. } | Outcome::NotFound) { "再チェック" } else { "チェック" };
                let button = egui::Button::new(RichText::new(label).strong());
                if ui.add_enabled(self.target.is_some(), button).clicked() {
                    self.run_check();
                }
            });
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| match &self.outcome {
            Outcome::Empty => {
                ui.centered_and_justified(|ui| ui.weak("まだチェックしていません"));
            }
            Outcome::Ready => {
                ui.centered_and_justified(|ui| ui.weak("「チェック」を押してください"));
            }
            Outcome::NotFound => {
                ui.colored_label(level_style(Level::Error).1, "talks/main.mnt が見つかりませんでした。");
                ui.label("ゴーストのフォルダ（ghost/master を含むフォルダ）か、talks フォルダの中の .mnt ファイルを選んでください。");
            }
            Outcome::Checked { master, result } => {
                ui.weak(format!("チェックしたフォルダ: {}", display_path(master)));
                if result.diagnostics.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("✔ エラーはありません").size(32.0).strong().color(Color32::from_rgb(0x20, 0xa0, 0x40)));
                    });
                    return;
                }
                let (e, w, n) = (count(result, Level::Error), count(result, Level::Warning), count(result, Level::Notice));
                ui.horizontal(|ui| {
                    if result.has_error() {
                        ui.label(RichText::new("修正箇所があります").size(18.0).strong().color(level_style(Level::Error).1));
                    } else {
                        ui.label(RichText::new("エラーはありません（気になる点があります）").size(18.0).strong());
                    }
                });
                ui.label(format!("エラー {}件 / 警告 {}件 / お知らせ {}件", e, w, n));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    for d in &result.diagnostics {
                        diagnostic_row(ui, d);
                    }
                });
            }
        });
    }
}
