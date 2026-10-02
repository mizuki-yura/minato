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

/// 位置の表示。「main.mntの4行目（OnBoot内）」のような形。位置もコンテキストも無ければ None。
pub fn location(d: &Diagnostic) -> Option<String> {
    let base = match (&d.file, d.line) {
        (Some(f), Some(l)) => Some(format!("{}の{}行目", f, l)),
        (None, Some(l)) => Some(format!("{}行目", l)),
        (Some(f), None) => Some(f.clone()),
        (None, None) => None,
    };
    match (base, &d.context) {
        (Some(b), Some(c)) => Some(format!("{}（{}内）", b, c)),
        (Some(b), None) => Some(b),
        (None, Some(c)) => Some(format!("{}内", c)),
        (None, None) => None,
    }
}

/// ErrorDescriptionヘッダ向け。改行を含まない1行を返す。
/// 例: 「main.mntの4行目（OnBoot内）: メッセージ ヒント: …」
pub fn render_header(d: &Diagnostic) -> String {
    let mut s = match location(d) {
        Some(loc) => format!("{}: {}", loc, d.message),
        None => d.message.clone(),
    };
    if let Some(h) = &d.hint {
        s.push_str(" ヒント: ");
        s.push_str(h);
    }
    // 1行ヘッダなので改行と、件数区切りの\x01を潰す
    s.replace(['\r', '\n', '\x01'], " ")
}

/// さくらスクリプトとして解釈されないよう、本文中の「\」をエスケープする。
fn escape_sakura(s: &str) -> String {
    s.replace(['\r', '\n'], " ").replace('\\', "\\\\")
}

/// バルーン本文向け（さくらスクリプト）。
/// プロポーショナルフォントで読まれるため桁揃えや下線には頼らず、
/// 位置とメッセージを1行、ヒントがあれば次の行に置く。
pub fn render_balloon(d: &Diagnostic) -> String {
    let mut s = match location(d) {
        Some(loc) => format!("{}: {}", escape_sakura(&loc), escape_sakura(&d.message)),
        None => escape_sakura(&d.message),
    };
    if let Some(h) = &d.hint {
        s.push_str("\\n");
        s.push_str("ヒント: ");
        s.push_str(&escape_sakura(h));
    }
    s
}

/// バルーン本文に出す最大件数。残りは「他n件」とまとめる。
pub const BALLOON_MAX_ITEMS: usize = 3;

/// ファイル名・行番号の順に並べる。位置の無いものは後ろ。順序の同じものは元の順を保つ。
pub fn sort_diagnostics(diags: &mut [Diagnostic]) {
    diags.sort_by(|a, b| {
        let key = |d: &Diagnostic| (d.file.is_none(), d.file.clone(), d.line.is_none(), d.line, d.col);
        key(a).cmp(&key(b))
    });
}

/// 複数件をバルーン本文にまとめる。先頭BALLOON_MAX_ITEMS件だけ表示し、
/// 残りは件数と全件の確認先を示す。並び替え済みの列を渡すこと。
pub fn render_balloon_list(diags: &[Diagnostic]) -> String {
    let mut parts: Vec<String> = diags.iter().take(BALLOON_MAX_ITEMS).map(render_balloon).collect();
    if diags.len() > BALLOON_MAX_ITEMS {
        parts.push(format!(
            "他{}件（minato_check で全件を確認できます）",
            diags.len() - BALLOON_MAX_ITEMS
        ));
    }
    parts.join("\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_format() {
        let d = Diagnostic::error("ループの外で break を使っています")
            .in_file("main.mnt").at(2).with_context("OnBoot")
            .with_hint("for/whileの中で使ってください");
        assert_eq!(
            render_header(&d),
            "main.mntの2行目（OnBoot内）: ループの外で break を使っています ヒント: for/whileの中で使ってください"
        );
        assert!(!render_header(&Diagnostic::error("a\nb\x01c")).contains(['\n', '\x01']));
    }

    #[test]
    fn balloon_format_with_hint() {
        let d = Diagnostic::error("1行目の「{」が閉じられていません")
            .in_file("main.mnt").at(1).with_hint("対応する「}」を書いてください");
        assert_eq!(
            render_balloon(&d),
            "main.mntの1行目: 1行目の「{」が閉じられていません\\nヒント: 対応する「}」を書いてください"
        );
    }

    #[test]
    fn balloon_escapes_backslash() {
        let d = Diagnostic::error("認識できない文です: 「\\0あ」").in_file("main.mnt").at(3);
        assert_eq!(render_balloon(&d), "main.mntの3行目: 認識できない文です: 「\\\\0あ」");
    }

    #[test]
    fn balloon_list_truncates_with_destination() {
        let diags: Vec<Diagnostic> = (1..=5)
            .map(|i| Diagnostic::error(format!("e{}", i)).in_file("main.mnt").at(i))
            .collect();
        let s = render_balloon_list(&diags);
        assert!(s.contains("e1") && s.contains("e3") && !s.contains("e4"), "{}", s);
        assert!(s.ends_with("\\n他2件（minato_check で全件を確認できます）"), "{}", s);
        assert!(!render_balloon_list(&diags[..3]).contains("他"));
    }

    #[test]
    fn sort_by_file_then_line() {
        let mut diags = vec![
            Diagnostic::error("c").in_file("sub.mnt").at(1),
            Diagnostic::error("nofile"),
            Diagnostic::error("b").in_file("main.mnt").at(9),
            Diagnostic::error("a").in_file("main.mnt").at(2),
        ];
        sort_diagnostics(&mut diags);
        let order: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(order, ["a", "b", "c", "nofile"]);
    }

    #[test]
    fn rendered_text_is_shift_jis_encodable() {
        let d = Diagnostic::error("1行目の「{」が閉じられていません")
            .in_file("main.mnt").at(1).with_context("OnBoot")
            .with_hint("次の可能性もあります: A／B");
        let many = vec![d.clone(); 5];
        for s in [render_header(&d), render_balloon_list(&many)] {
            let (_, _, had_errors) = encoding_rs::SHIFT_JIS.encode(&s);
            assert!(!had_errors, "Shift_JISに無い文字を含む: {}", s);
        }
    }

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
