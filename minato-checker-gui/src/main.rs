// main.rs
// 湊（minato）のGUI構文チェッカー。
// ゴーストのフォルダか .mnt ファイルをドロップ（またはダイアログで選択）すると、
// CLIのminato_checkと同じ minato::checker::check_ghost を走らせて結果を一覧表示する。
// エラー文言はminato本体のDiagnosticをそのまま使い、ここでは組み立てない。

// Windowsでダブルクリック起動したときにコンソール窓を出さない（他のOSでは無視される）
#![cfg_attr(windows, windows_subsystem = "windows")]

mod shell_warmup;
#[cfg(windows)]
mod win_drop;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText};
use minato::checker::{check_ghost, resolve_master_dir, CheckResult};
use minato::diagnostic::{Diagnostic, Level};

/// 前回選んだフォルダを保存するキー（eframeの設定ファイルに入る）
const LAST_DIR_KEY: &str = "last_dir";

const GUIDE: &str = "ゴーストのフォルダ（または .mnt ファイル）を、この窓にドラッグ＆ドロップしてください。\n\
ドロップできないときは「フォルダを選ぶ」「ファイルを選ぶ」ボタンから選べます。\n\
書いたトークに書き間違いがないかを、SSPを起動せずに確かめられます。";

fn main() -> eframe::Result {
    shell_warmup::spawn();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("湊 構文チェッカー")
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([480.0, 360.0])
            // Windowsではwinitのドロップを使わず、win_dropで受け取る（理由はwin_drop.rs）
            .with_drag_and_drop(!cfg!(windows)),
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
    /// チェック中。結果は別スレッドから届く
    Checking(Receiver<Outcome>),
    /// talks/main.mnt が見つからなかった
    NotFound,
    Checked { master: PathBuf, result: CheckResult },
}

/// 画面の状態。
struct App {
    ctx: egui::Context,
    /// 前回選んだフォルダ。選択画面はここから開く
    last_dir: Option<PathBuf>,
    font_ok: bool,
    /// Windowsでドロップされたパスの受け取り口（win_drop）
    dropped: Option<Receiver<PathBuf>>,
    /// 開いているファイル・フォルダ選択画面の結果の受け取り口（開いていなければNone）
    dialog: Option<Receiver<Option<PathBuf>>>,
    target: Option<PathBuf>,
    outcome: Outcome,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let font_ok = install_japanese_font(&cc.egui_ctx);
        let last_dir = cc
            .storage
            .and_then(|s| s.get_string(LAST_DIR_KEY))
            .map(PathBuf::from);
        let mut app = Self {
            ctx: cc.egui_ctx.clone(),
            last_dir,
            font_ok,
            dropped: None,
            dialog: None,
            target: None,
            outcome: Outcome::Empty,
        };
        #[cfg(windows)]
        {
            app.dropped = win_drop::install(cc, &cc.egui_ctx);
        }
        // exeのアイコンにフォルダをドロップして起動したときは、それをすぐチェックする
        if let Some(arg) = std::env::args_os().nth(1) {
            app.select(PathBuf::from(arg), true);
        }
        app
    }

    fn select(&mut self, path: PathBuf, run_now: bool) {
        // .mntが選ばれたときはそれがあるフォルダを覚える
        let dir = if path.is_file() { path.parent().map(Path::to_path_buf) } else { Some(path.clone()) };
        if dir.is_some() {
            self.last_dir = dir;
        }
        self.target = Some(path);
        self.outcome = Outcome::Ready;
        if run_now {
            self.run_check();
        }
    }

    /// 大きいゴーストだと数秒かかるため、窓が固まらないよう別スレッドでチェックする。
    /// チェック中に別のフォルダが選ばれたら、古い結果は受け取り手がいなくなり捨てられる。
    fn run_check(&mut self) {
        let Some(target) = self.target.clone() else { return };
        let (tx, rx) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let outcome = match resolve_master_dir(&target) {
                Some(master) => {
                    let result = check_ghost(&master);
                    Outcome::Checked { master, result }
                }
                None => Outcome::NotFound,
            };
            let _ = tx.send(outcome);
            ctx.request_repaint();
        });
        self.outcome = Outcome::Checking(rx);
    }

    /// 選択画面を別スレッドで開く。初回はWindowsがエクスプローラーの部品を読み込むため時間がかかり、
    /// UIのスレッドで開くとその間この窓が止まって「応答待ち」のカーソルが出てしまう。
    fn open_dialog(&mut self, pick: impl FnOnce(rfd::FileDialog) -> Option<PathBuf> + Send + 'static) {
        let mut dialog = rfd::FileDialog::new();
        // 前回のフォルダが消えていたら、Windowsの既定の場所から開く
        if let Some(dir) = self.last_dir.as_ref().filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        let (tx, rx) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(pick(dialog));
            ctx.request_repaint();
        });
        self.dialog = Some(rx);
    }

    /// 選択画面が閉じていたら、選ばれたものをチェックする。
    fn poll_dialog(&mut self) {
        let Some(rx) = &self.dialog else { return };
        match rx.try_recv() {
            Ok(picked) => {
                self.dialog = None;
                if let Some(p) = picked {
                    self.select(p, true);
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.dialog = None,
        }
    }

    /// 別スレッドのチェックが終わっていたら結果を受け取る。
    fn poll_check(&mut self) {
        if let Outcome::Checking(rx) = &self.outcome {
            match rx.try_recv() {
                Ok(outcome) => self.outcome = outcome,
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.outcome = Outcome::Ready;
                }
            }
        }
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
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(dir) = &self.last_dir {
            storage.set_string(LAST_DIR_KEY, dir.to_string_lossy().into_owned());
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_dialog();
        self.poll_check();
        let dialog_open = self.dialog.is_some();
        let checking = matches!(self.outcome, Outcome::Checking(_));
        // ドロップされたら、そのまま1回チェックする
        let dropped: Option<PathBuf> = match &self.dropped {
            Some(rx) => rx.try_iter().last(),
            None => ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone())),
        };
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
                if ui.add_enabled(!dialog_open, egui::Button::new("フォルダを選ぶ")).clicked() {
                    self.open_dialog(|d| d.pick_folder());
                }
                if ui.add_enabled(!dialog_open, egui::Button::new("ファイルを選ぶ")).clicked() {
                    self.open_dialog(|d| d.add_filter("湊スクリプト", &["mnt"]).pick_file());
                }
                if dialog_open {
                    ui.spinner();
                    ui.label("選択画面を開いています…");
                }
                ui.separator();
                let label = if matches!(self.outcome, Outcome::Checked { .. } | Outcome::NotFound) { "再チェック" } else { "チェック" };
                let button = egui::Button::new(RichText::new(label).strong());
                if ui.add_enabled(self.target.is_some() && !checking, button).clicked() {
                    self.run_check();
                }
            });
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| match &self.outcome {
            Outcome::Empty => {
                ui.centered_and_justified(|ui| ui.weak("まだチェックしていません"));
            }
            Outcome::Checking(_) => {
                ui.centered_and_justified(|ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("チェック中…").size(18.0));
                    });
                });
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
