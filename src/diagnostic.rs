// diagnostic.rs
// 構文エラー・静的解析結果などの診断情報を、出力先に依存しない形で表す。
// 文字列への組み立てはレンダラー（render_*）に任せ、
// バルーン・CLI・将来のGUIチェッカーで同じDiagnosticを使い回す。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    Notice,
}

impl Level {
    /// SHIORIのErrorLevelヘッダや「[error]」表示に使う文字列。
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Notice => "notice",
        }
    }
}

/// 1件の診断。
/// line/col は「line が無いのに col だけある」状態を作らないよう非公開にし、
/// at / at_col 経由でのみ設定する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: Level,
    /// ファイル名（main.mnt など）。位置を特定できないエラーでは None。
    pub file: Option<String>,
    line: Option<u32>,
    col: Option<u32>,
    /// 「OnBoot」のようなイベント名・関数名。
    pub context: Option<String>,
    pub message: String,
    pub hint: Option<String>,
}

impl Diagnostic {
    pub fn new(level: Level, message: impl Into<String>) -> Self {
        Self {
            level,
            file: None,
            line: None,
            col: None,
            context: None,
            message: message.into(),
            hint: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Level::Error, message)
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Level::Warning, message)
    }

    pub fn notice(message: impl Into<String>) -> Self {
        Self::new(Level::Notice, message)
    }

    /// 空文字のファイル名は「ファイル不明」として扱う。
    pub fn in_file(mut self, file: impl Into<String>) -> Self {
        let f = file.into();
        self.file = if f.is_empty() { None } else { Some(f) };
        self
    }

    /// 行番号を設定する。桁は不明として消す。
    pub fn at(mut self, line: u32) -> Self {
        self.line = Some(line);
        self.col = None;
        self
    }

    /// 行番号と桁（1-indexed、文字単位）を設定する。
    pub fn at_col(mut self, line: u32, col: u32) -> Self {
        self.line = Some(line);
        self.col = Some(col);
        self
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        let c = context.into();
        self.context = if c.is_empty() { None } else { Some(c) };
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn line(&self) -> Option<u32> {
        self.line
    }

    pub fn col(&self) -> Option<u32> {
        self.col
    }
}

// ── レンダラー ───────────────────────────────────────────

/// 従来の文字列形式（ErrorDescriptionやログ向け）。改行を含まない1行を返す。
/// - 位置あり: 「main.mntの4行目: メッセージ」
/// - コンテキストあり: 「OnBoot内 main.mntの4行目: メッセージ」
/// - 位置なし: 「メッセージ」
pub fn render_legacy(d: &Diagnostic) -> String {
    let mut s = String::new();
    if let Some(ctx) = &d.context {
        s.push_str(ctx);
        s.push_str("内 ");
    }
    match (&d.file, d.line) {
        (Some(f), Some(l)) => s.push_str(&format!("{}の{}行目: ", f, l)),
        (None, Some(l)) => s.push_str(&format!("{}行目: ", l)),
        (Some(f), None) => s.push_str(&format!("{}: ", f)),
        (None, None) => {}
    }
    s.push_str(&d.message);
    if let Some(h) = &d.hint {
        s.push(' ');
        s.push_str(h);
    }
    // ErrorDescriptionは1行ヘッダなので念のため改行を潰す
    s.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_parse_error_format() {
        let d = Diagnostic::error("式が必要です").in_file("main.mnt").at(4);
        assert_eq!(render_legacy(&d), "main.mntの4行目: 式が必要です");
    }

    #[test]
    fn legacy_analyze_format_with_context() {
        let d = Diagnostic::error("ループの外で break を使っています")
            .in_file("main.mnt").at(2).with_context("OnBoot");
        assert_eq!(render_legacy(&d), "OnBoot内 main.mntの2行目: ループの外で break を使っています");
    }

    #[test]
    fn legacy_analyze_format_without_file_matches_old() {
        // ファイル名が無い場合は従来の「OnBoot内 2行目: …」と一致する
        let d = Diagnostic::error("ループの外で break を使っています")
            .in_file("").at(2).with_context("OnBoot");
        assert_eq!(render_legacy(&d), "OnBoot内 2行目: ループの外で break を使っています");
    }

    #[test]
    fn legacy_no_location() {
        let d = Diagnostic::error("パース中に内部エラーが発生しました。");
        assert_eq!(render_legacy(&d), "パース中に内部エラーが発生しました。");
    }

    #[test]
    fn at_clears_col() {
        let d = Diagnostic::error("x").at_col(3, 5).at(4);
        assert_eq!((d.line(), d.col()), (Some(4), None));
    }

    #[test]
    fn legacy_has_no_newline() {
        let d = Diagnostic::error("a\nb").with_hint("c\r\nd");
        assert!(!render_legacy(&d).contains('\n'));
    }
}
