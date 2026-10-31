// analyzer.rs
// ASTの静的チェック
use std::collections::{HashMap, HashSet};
use crate::parser::{Stmt, Expr, Talk, Spanned, StrPart, PathSegment, MatchPattern, AssignOp};
use crate::codegen::is_builtin;
use crate::diagnostic::{Diagnostic, Level};

/// 診断を出す位置（ファイル名と元ソースの行番号）。
#[derive(Clone, Copy)]
struct Pos<'b> {
    file: &'b str,
    line: u32,
}

impl<'b> Pos<'b> {
    fn of(spanned: &'b Spanned<Stmt>) -> Self {
        Pos { file: &spanned.file, line: spanned.line }
    }
}

pub struct Analyzer<'a> {
    /// 定義済みtalk名
    talk_names: HashSet<String>,
    /// 定義済みfunc名（トップレベル＋現在の検査対象内でローカル定義されたもの）
    func_names: HashSet<String>,
    errors: Vec<Diagnostic>,
    loop_depth: usize,
    current_context: &'a str,
    /// globalとして宣言された名前（パスの先頭）。トップレベル＋talk/func内のglobal文
    global_names: HashSet<String>,
    /// 変数チェック用のスコープ（内側が末尾）。名前ごとに宣言位置と読み書きを記録する
    let_scopes: Vec<HashMap<String, Binding>>,
    /// 自由変数チェックで報告済みの名前（talk/funcごとにリセット）
    reported_reads: HashSet<String>,
    reported_assigns: HashSet<String>,
}

/// スコープ内の変数1つ分の記録。未使用・書くだけの変数の検出に使う。
struct Binding {
    /// let で作った変数か（引数・ループ変数は未使用でも報告しない）
    is_let: bool,
    file: String,
    line: u32,
    read: bool,
    written: bool,
}

/// 静的チェックのルールID。表示とドキュメントで使う。
pub mod rule {
    pub const FREE_VARIABLE: &str = "free-variable";
    pub const UNUSED_VARIABLE: &str = "unused-variable";
    pub const WRITE_ONLY_VARIABLE: &str = "write-only-variable";
    pub const UNREACHABLE_CODE: &str = "unreachable-code";
    pub const UNDEFINED_CALL: &str = "undefined-call";
    pub const LOOP_CONTROL_OUTSIDE_LOOP: &str = "loop-control-outside-loop";
}

/// 処理系が用意する変数。引数・let・globalのどれでもなくても読める。
/// reference/now はイベントごとのローカル、system/status はグローバル。
const AMBIENT_NAMES: &[&str] = &["reference", "now", "system", "status"];

/// トップレベルの global 文から、宣言された名前（パスの先頭）を集める。
pub fn top_global_names(globals: &[(Vec<PathSegment>, AssignOp, Expr)]) -> HashSet<String> {
    globals.iter().filter_map(|(path, _, _)| path_head(path)).collect()
}

fn path_head(path: &[PathSegment]) -> Option<String> {
    match path.first() {
        Some(PathSegment::Key(k)) => Some(k.clone()),
        _ => None,
    }
}

impl<'a> Analyzer<'a> {
    pub fn new(
        talk_names: HashSet<String>,
        func_names: HashSet<String>,
    ) -> Self {
        Self {
            talk_names,
            func_names,
            errors: vec![],
            loop_depth: 0,
            current_context: "",
            global_names: HashSet::new(),
            let_scopes: vec![],
            reported_reads: HashSet::new(),
            reported_assigns: HashSet::new(),
        }
    }

    /// トップレベルの global 文で宣言された名前を渡す。
    /// talk/func内の global 文はanalyzeの中で集める。
    pub fn with_global_names(mut self, names: HashSet<String>) -> Self {
        self.global_names.extend(names);
        self
    }

pub fn analyze(
    mut self,
    talks: &'a HashMap<String, Vec<Talk>>,
    funcs: &'a [(String, Vec<String>, Vec<Spanned<Stmt>>)],
) -> Vec<Diagnostic> {
    // talkを検査
    // HashMapの走査順は非決定的（実行のたびに変わりうる）なため、
    // エラーメッセージの並びを安定させるためイベント名でソートしてから走査する。
    let mut talk_names_sorted: Vec<&String> = talks.keys().collect();
    talk_names_sorted.sort();

    // talk/func内の global 文で宣言された名前も、どこからでも読めるglobalとして扱う
    for talk in talks.values().flatten() {
        collect_global_names_into(&talk.body, &mut self.global_names);
    }
    for (_, _, body) in funcs {
        collect_global_names_into(body, &mut self.global_names);
    }

   for event_name in talk_names_sorted {
        let talk_list = &talks[event_name];

        for talk in talk_list {
            self.current_context = &talk.event;
            self.loop_depth = 0;
            // condは本体より先に評価されるので、本体のローカルfuncを登録する前に検査する
            if let Some(cond) = &talk.cond {
                self.check_expr(Pos { file: &talk.file, line: talk.cond_line }, cond);
            }
            let local_funcs = collect_local_func_names(&talk.body);
            for name in &local_funcs { self.func_names.insert(name.clone()); }
            self.check_stmts(&talk.body);
            self.begin_free_var_check(&[]);
            if let Some(cond) = &talk.cond {
                self.fv_expr(Pos { file: &talk.file, line: talk.cond_line }, cond);
            }
            self.fv_stmts(&talk.body);
            self.end_free_var_check();
            for name in &local_funcs { self.func_names.remove(name); }
        }
    }
    // トップレベルfuncを検査（Vec由来なので元々順序は安定）
    for (name, params, body) in funcs {
        self.current_context = name;
        self.loop_depth = 0;
        let local_funcs = collect_local_func_names(body);
        for n in &local_funcs { self.func_names.insert(n.clone()); }
        self.check_stmts(body);
        self.begin_free_var_check(params);
        self.fv_stmts(body);
        self.end_free_var_check();
        for n in &local_funcs { self.func_names.remove(n); }
    }
    self.errors
}

    // ---- 自由変数チェック ----
    // func/talkの中から見えるのは、引数・自分のlet・globalと reference/now だけ。
    // どれでもない名前を読むと null になり、let なしで代入するとglobalを新しく作る。
    // 0.2系までは呼び出し元のletが見えていたので、それに頼っていた台本や
    // 書き間違いを見つけるため、そうした名前にnoticeを出す。

    fn begin_free_var_check(&mut self, params: &[String]) {
        self.let_scopes = vec![HashMap::new()];
        for p in params { self.declare_other(p); }
        self.reported_reads.clear();
        self.reported_assigns.clear();
    }

    /// talk/funcの検査の終わりに、残っているスコープをすべて閉じる。
    fn end_free_var_check(&mut self) {
        while !self.let_scopes.is_empty() { self.pop_let_scope(); }
    }

    fn push_let_scope(&mut self) {
        self.let_scopes.push(HashMap::new());
    }

    /// スコープを閉じ、その中で let した変数のうち読まれなかったものを報告する。
    fn pop_let_scope(&mut self) {
        if let Some(scope) = self.let_scopes.pop() {
            let mut names: Vec<(String, Binding)> = scope.into_iter().collect();
            // HashMapの順序は非決定的なので、宣言位置で並べて報告順を安定させる
            names.sort_by(|a, b| (&a.1.file, a.1.line, &a.0).cmp(&(&b.1.file, b.1.line, &b.0)));
            for (name, b) in names { self.report_unread(&name, &b); }
        }
    }

    fn report_unread(&mut self, name: &str, b: &Binding) {
        if !b.is_let || b.read || name.starts_with('_') { return; }
        let at = Pos { file: &b.file, line: b.line };
        let (rule_id, msg) = if b.written {
            (rule::WRITE_ONLY_VARIABLE, format!(
                "\"{}\" に値を入れていますが、一度も読んでいません（要らないなら消すか、名前を _ で始めてください）",
                name
            ))
        } else {
            (rule::UNUSED_VARIABLE, format!(
                "\"{}\" は let で作られていますが、一度も使われていません（要らないなら消すか、名前を _ で始めてください）",
                name
            ))
        };
        // Posは&strを借りるので、bを借りたままpush_atを呼べるよう値を作ってから渡す
        let diag = Diagnostic::new(Level::Notice, msg)
            .in_file(at.file)
            .at(at.line)
            .with_context(self.current_context)
            .with_rule(rule_id);
        self.errors.push(diag);
    }

    /// nameが見えている一番内側の記録。
    fn binding_mut(&mut self, name: &str) -> Option<&mut Binding> {
        self.let_scopes.iter_mut().rev().find_map(|s| s.get_mut(name))
    }

    fn is_bound(&self, name: &str) -> bool {
        self.let_scopes.iter().any(|s| s.contains_key(name))
            || self.global_names.contains(name)
            || AMBIENT_NAMES.contains(&name)
            || self.func_names.contains(name)
            || self.talk_names.contains(name)
    }

    /// let で変数を作る。同じスコープで同じ名前をもう一度 let したときは、
    /// 前の変数はそこで見えなくなるので、読まれていなければここで報告する。
    fn declare_let(&mut self, at: Pos<'_>, name: &str) {
        let binding = Binding { is_let: true, file: at.file.to_string(), line: at.line, read: false, written: false };
        let prev = self.let_scopes.last_mut().and_then(|s| s.insert(name.to_string(), binding));
        if let Some(prev) = prev { self.report_unread(name, &prev); }
    }

    /// 引数・ループ変数。未使用でも報告しない。
    fn declare_other(&mut self, name: &str) {
        if let Some(scope) = self.let_scopes.last_mut() {
            scope.insert(name.to_string(), Binding { is_let: false, file: String::new(), line: 0, read: false, written: false });
        }
    }

    fn fv_read(&mut self, at: Pos<'_>, name: &str) {
        if let Some(b) = self.binding_mut(name) {
            b.read = true;
            return;
        }
        if !self.is_bound(name) && self.reported_reads.insert(name.to_string()) {
            self.push_at(Level::Notice, rule::FREE_VARIABLE, at, format!(
                "\"{}\" は引数・let・global のどれでもないので、null になります。呼び出し元の let は見えません（セーブデータにだけ残る変数なら無視してください）",
                name
            ));
        }
    }

    fn fv_assign(&mut self, at: Pos<'_>, name: &str) {
        if let Some(b) = self.binding_mut(name) {
            b.written = true;
            return;
        }
        if !self.is_bound(name) && self.reported_assigns.insert(name.to_string()) {
            self.push_at(Level::Notice, rule::FREE_VARIABLE, at, format!(
                "\"{}\" への代入は、グローバル変数を新しく作ります。呼び出し元の変数は書き換わりません（let か global を付けてください）",
                name
            ));
        }
    }

    fn fv_stmts(&mut self, stmts: &[Spanned<Stmt>]) {
        for spanned in stmts {
            self.fv_stmt(Pos::of(spanned), &spanned.node);
        }
    }

    /// 実行時と同じく新しいletスコープを積んで文リストを検査する。
    fn fv_scoped(&mut self, stmts: &[Spanned<Stmt>]) {
        self.push_let_scope();
        self.fv_stmts(stmts);
        self.pop_let_scope();
    }

    fn fv_path_indices(&mut self, at: Pos<'_>, path: &[PathSegment]) {
        for seg in path {
            if let PathSegment::Index(e) = seg { self.fv_expr(at, e); }
        }
    }

    /// for の init/step はSpannedでない Stmt なので、Pos を外から受け取る。
    fn fv_stmt(&mut self, at: Pos<'_>, stmt: &Stmt) {
        match stmt {
            Stmt::Dialogue(line) => self.fv_str_parts(at, &line.content),
            // 右辺は束縛より先に評価される（let x = x + 1 の右辺のxは外側のx）
            Stmt::Let(name, e) => {
                self.fv_expr(at, e);
                self.declare_let(at, name);
            }
            Stmt::Global(path, _, e) => {
                self.fv_path_indices(at, path);
                self.fv_expr(at, e);
            }
            Stmt::Assign(path, _, e) => {
                self.fv_path_indices(at, path);
                self.fv_expr(at, e);
                if let Some(head) = path_head(path) { self.fv_assign(at, &head); }
            }
            Stmt::If(cond, then_body, else_body) => {
                self.fv_expr(at, cond);
                self.fv_scoped(then_body);
                if let Some(eb) = else_body { self.fv_scoped(eb); }
            }
            Stmt::While(cond, body) => {
                self.fv_expr(at, cond);
                self.fv_scoped(body);
            }
            // initのletはループ全体を囲むスコープに入る
            Stmt::For { init, cond, step, body } => {
                self.push_let_scope();
                self.fv_stmt(at, init);
                self.fv_expr(at, cond);
                self.fv_scoped(body);
                self.fv_for_step(at, step);
                self.pop_let_scope();
            }
            Stmt::ForEach { collection, key, value, body } => {
                self.fv_expr(at, collection);
                self.push_let_scope();
                self.declare_other(key);
                if let Some(v) = value { self.declare_other(v); }
                self.fv_stmts(body);
                self.pop_let_scope();
            }
            Stmt::Match { expr, arms } => {
                self.fv_expr(at, expr);
                for arm in arms {
                    for p in &arm.patterns {
                        if let MatchPattern::Value(e) = p { self.fv_expr(at, e); }
                    }
                    self.fv_scoped(&arm.body);
                }
            }
            Stmt::Return(e) => self.fv_expr(at, e),
            Stmt::Call(Expr::Call(_, args)) => {
                for a in args { self.fv_expr(at, a); }
            }
            // call 名前 の名前はふつうトーク名なので、自由変数としては報告しない。
            // ただし let した変数なら、その値をトーク名に使う動的callなので読みとして数える
            Stmt::Call(Expr::Var(path)) => {
                if let Some(b) = path.first().and_then(|head| self.binding_mut(head)) {
                    b.read = true;
                }
            }
            // ローカル定義のfuncは外側のletを捕まえないので、
            // 外側とは切り離し、引数だけを束縛として検査する
            Stmt::FuncDef { params, body, .. } => {
                let outer_scopes = std::mem::take(&mut self.let_scopes);
                let outer_reads = std::mem::take(&mut self.reported_reads);
                let outer_assigns = std::mem::take(&mut self.reported_assigns);
                self.begin_free_var_check(params);
                self.fv_stmts(body);
                self.end_free_var_check();
                self.let_scopes = outer_scopes;
                self.reported_reads = outer_reads;
                self.reported_assigns = outer_assigns;
            }
            Stmt::Call(_) | Stmt::Break | Stmt::Continue => {}
        }
    }

    /// for の step（i++ 等）。パーサーはGlobal文として作るが、
    /// 実行時は let と同じくスコープをさかのぼって代入する（step_for）。
    fn fv_for_step(&mut self, at: Pos<'_>, step: &Stmt) {
        if let Stmt::Global(path, _, e) = step {
            self.fv_path_indices(at, path);
            self.fv_expr(at, e);
            if let Some(head) = path_head(path) { self.fv_assign(at, &head); }
        } else {
            self.fv_stmt(at, step);
        }
    }

    fn fv_expr(&mut self, at: Pos<'_>, expr: &Expr) {
        match expr {
            Expr::Var(path) => {
                if let Some(head) = path.first() { self.fv_read(at, head); }
            }
            Expr::InterpolatedStr(parts) => self.fv_str_parts(at, parts),
            Expr::Call(_, args) => {
                for a in args { self.fv_expr(at, a); }
            }
            Expr::Array(items) => {
                for e in items { self.fv_expr(at, e); }
            }
            Expr::Map(pairs) => {
                for (k, v) in pairs {
                    self.fv_expr(at, k);
                    self.fv_expr(at, v);
                }
            }
            Expr::BinOp(l, _, r)
            | Expr::Cmp(l, _, r)
            | Expr::And(l, r)
            | Expr::Or(l, r)
            | Expr::NullCoalesce(l, r)
            | Expr::Index(l, r) => {
                self.fv_expr(at, l);
                self.fv_expr(at, r);
            }
            Expr::Not(e) => self.fv_expr(at, e),
            Expr::Str(_) | Expr::Number(_) | Expr::Bool(_) | Expr::Null => {}
        }
    }

    fn fv_str_parts(&mut self, at: Pos<'_>, parts: &[StrPart]) {
        for p in parts {
            match p {
                StrPart::Var(path) => {
                    if let Some(head) = path.first() { self.fv_read(at, head); }
                }
                StrPart::Expr(e) => self.fv_expr(at, e),
                StrPart::Lit(_) => {}
            }
        }
    }

    fn push(&mut self, level: Level, rule: &'static str, spanned: &Spanned<Stmt>, message: impl Into<String>) {
        self.push_at(level, rule, Pos::of(spanned), message);
    }

    fn push_at(&mut self, level: Level, rule: &'static str, at: Pos<'_>, message: impl Into<String>) {
        self.errors.push(
            Diagnostic::new(level, message)
                .in_file(at.file)
                .at(at.line)
                .with_context(self.current_context)
                .with_rule(rule),
        );
    }

    fn check_stmts(&mut self, stmts: &[Spanned<Stmt>]) {
        // return/break/continue より後ろの文は実行されない。ブロックごとに最初の1文だけ報告する
        let mut jumped_by: Option<&str> = None;
        for spanned in stmts {
            if let Some(kw) = jumped_by.take() {
                self.push(Level::Warning, rule::UNREACHABLE_CODE, spanned, format!(
                    "この行は {} より後ろにあるので、実行されません", kw
                ));
            }
            self.check_stmt(spanned);
            if jumped_by.is_none() {
                jumped_by = match spanned.node {
                    Stmt::Return(_) => Some("return"),
                    Stmt::Break => Some("break"),
                    Stmt::Continue => Some("continue"),
                    _ => None,
                };
            }
        }
    }

    /// 式の中の関数呼び出し `名前(...)` を再帰的にたどり、未定義の名前に notice を出す。
    /// 実行時は func → talk → ビルトインの順に探すので、そのどれでもなければ未定義。
    fn check_expr(&mut self, at: Pos<'_>, expr: &Expr) {
        match expr {
            Expr::Call(name, args) => {
                self.check_call_name(at, name);
                for a in args { self.check_expr(at, a); }
            }
            Expr::Array(items) => {
                for e in items { self.check_expr(at, e); }
            }
            Expr::Map(pairs) => {
                for (k, v) in pairs {
                    self.check_expr(at, k);
                    self.check_expr(at, v);
                }
            }
            Expr::InterpolatedStr(parts) => self.check_str_parts(at, parts),
            Expr::BinOp(l, _, r)
            | Expr::Cmp(l, _, r)
            | Expr::And(l, r)
            | Expr::Or(l, r)
            | Expr::NullCoalesce(l, r)
            | Expr::Index(l, r) => {
                self.check_expr(at, l);
                self.check_expr(at, r);
            }
            Expr::Not(e) => self.check_expr(at, e),
            Expr::Str(_) | Expr::Number(_) | Expr::Bool(_) | Expr::Null | Expr::Var(_) => {}
        }
    }

    fn check_str_parts(&mut self, at: Pos<'_>, parts: &[StrPart]) {
        for p in parts {
            if let StrPart::Expr(e) = p { self.check_expr(at, e); }
        }
    }

    fn check_path(&mut self, at: Pos<'_>, path: &[PathSegment]) {
        for seg in path {
            if let PathSegment::Index(e) = seg { self.check_expr(at, e); }
        }
    }

    fn check_call_name(&mut self, at: Pos<'_>, name: &str) {
        if !is_builtin(name)
            && !self.talk_names.contains(name)
            && !self.func_names.contains(name)
        {
            self.push_at(Level::Notice, rule::UNDEFINED_CALL, at, format!(
                    "\"{}\" は未定義です（ビルトイン関数でも talk でも func でもありません）",
                    name
                ));
        }
    }

    /// 文が直接持つ式（ネストしたブロックの中は除く）を検査する。
    /// for の init/step はSpannedでない Stmt なので、外側の文の行で報告する。
    fn check_stmt_exprs(&mut self, at: Pos<'_>, stmt: &Stmt) {
        match stmt {
            Stmt::Dialogue(line) => self.check_str_parts(at, &line.content),
            Stmt::Let(_, e) | Stmt::Return(e) | Stmt::While(e, _) | Stmt::If(e, _, _) => {
                self.check_expr(at, e);
            }
            Stmt::Global(path, _, e) | Stmt::Assign(path, _, e) => {
                self.check_path(at, path);
                self.check_expr(at, e);
            }
            Stmt::For { init, cond, step, .. } => {
                self.check_stmt_exprs(at, init);
                self.check_expr(at, cond);
                self.check_stmt_exprs(at, step);
            }
            Stmt::ForEach { collection, .. } => self.check_expr(at, collection),
            Stmt::Match { expr, arms } => {
                self.check_expr(at, expr);
                for arm in arms {
                    for p in &arm.patterns {
                        if let MatchPattern::Value(e) = p { self.check_expr(at, e); }
                    }
                }
            }
            // 呼び出す名前そのものは check_stmt の Stmt::Call で見るので、ここでは引数だけ
            Stmt::Call(Expr::Call(_, args)) => {
                for a in args { self.check_expr(at, a); }
            }
            Stmt::Call(_) | Stmt::FuncDef { .. } | Stmt::Break | Stmt::Continue => {}
        }
    }

    fn check_stmt(&mut self, spanned: &Spanned<Stmt>) {
        let stmt = &spanned.node;
        self.check_stmt_exprs(Pos::of(spanned), stmt);
        match stmt {
            Stmt::Break => {
                if self.loop_depth == 0 {
                    self.push(Level::Error, rule::LOOP_CONTROL_OUTSIDE_LOOP, spanned, "ループの外で break を使っています".to_string());
                }
            }
            Stmt::Continue => {
                if self.loop_depth == 0 {
                    self.push(Level::Error, rule::LOOP_CONTROL_OUTSIDE_LOOP, spanned, "ループの外で continue を使っています".to_string());
                }
            }
           Stmt::Return(_) => {

}

Stmt::Call(expr) => {
    match expr {
        Expr::Var(path) if path.len() == 1 => {
            let name = &path[0];
            if !self.talk_names.contains(name) && !self.func_names.contains(name) {
                self.push(Level::Notice, rule::UNDEFINED_CALL, spanned, format!(
                        "\"{}\" は未定義です（動的callなら無視してください）",
                        name
                    ));
            }
        }
        Expr::Var(path) if path.len() >= 2 => {
            // call reference.0 のように変数の値をイベント名として使う動的callが広く使われているため、
            // ドット付きpathを一律に未定義扱いすると誤爆する。
            // 「同じ path[0] + "." で始まるトーク群が定義されているのに該当名だけ無い」場合に限り、
            // 打ち間違いの可能性が高いとして notice を出す。
            let joined = path.join(".");
            if !self.talk_names.contains(&joined) {
                let prefix = format!("{}.", path[0]);
                if self.talk_names.iter().any(|n| n.starts_with(&prefix)) {
                    self.push(Level::Notice, rule::UNDEFINED_CALL, spanned, format!(
                            "\"{}\" は未定義です（同じ「{}」で始まるトークはありますが、この名前はありません）",
                            joined, prefix
                        ));
                }
            }
        }
        Expr::Call(name, _) => self.check_call_name(Pos::of(spanned), name),
        _ => {}
    }
}
            Stmt::If(_, then_body, else_body) => {
                self.check_stmts(then_body);
                if let Some(eb) = else_body {
                    self.check_stmts(eb);
                }
            }
            Stmt::While(_, body) => {
                self.loop_depth += 1;
                self.check_stmts(body);
                self.loop_depth -= 1;
            }
            Stmt::For { body, .. } => {
                self.loop_depth += 1;
                self.check_stmts(body);
                self.loop_depth -= 1;
            }
            Stmt::ForEach { body, .. } => {
                self.loop_depth += 1;
                self.check_stmts(body);
                self.loop_depth -= 1;
            }
            Stmt::Match { arms, .. } => {
                for arm in arms {
                    self.check_stmts(&arm.body);
                }
            }
            Stmt::FuncDef { body, .. } => {
                // func内のネスト定義（現状パーサーでは出ないが念のため）
                let prev_loop = self.loop_depth;
                self.loop_depth = 0;
                self.check_stmts(body);
                self.loop_depth = prev_loop;
            }
            // 以下は再帰不要
            Stmt::Dialogue(_)
            | Stmt::Let(_, _)
            | Stmt::Global(_, _, _)
            | Stmt::Assign(_, _, _) => {}
        }
    }
}

/// 文リスト直下（ネストしたブロックの中も含む）で定義されているFuncDefの名前を集める。
/// analyze実行前に1パス走査して、talk/func本体内でローカル定義される関数名を
/// あらかじめ把握するために使う。
fn collect_local_func_names(stmts: &[Spanned<Stmt>]) -> HashSet<String> {
    let mut names = HashSet::new();
    collect_local_func_names_into(stmts, &mut names);
    names
}

fn collect_local_func_names_into(stmts: &[Spanned<Stmt>], names: &mut HashSet<String>) {
    for spanned in stmts {
        match &spanned.node {
            Stmt::FuncDef { name, body, .. } => {
                names.insert(name.clone());
                collect_local_func_names_into(body, names);
            }
            Stmt::If(_, then_body, else_body) => {
                collect_local_func_names_into(then_body, names);
                if let Some(eb) = else_body {
                    collect_local_func_names_into(eb, names);
                }
            }
            Stmt::While(_, body)
            | Stmt::For { body, .. }
            | Stmt::ForEach { body, .. } => {
                collect_local_func_names_into(body, names);
            }
            Stmt::Match { arms, .. } => {
                for arm in arms {
                    collect_local_func_names_into(&arm.body, names);
                }
            }
            _ => {}
        }
    }
}

/// 文リスト（ネストしたブロック・ローカルfuncの中も含む）の global 文から、
/// 宣言された名前（パスの先頭）を集める。
/// for の step はパーサー上Global文だが実際はローカルへの代入なので含めない。
fn collect_global_names_into(stmts: &[Spanned<Stmt>], names: &mut HashSet<String>) {
    for spanned in stmts {
        match &spanned.node {
            Stmt::Global(path, _, _) => {
                if let Some(head) = path_head(path) { names.insert(head); }
            }
            Stmt::If(_, then_body, else_body) => {
                collect_global_names_into(then_body, names);
                if let Some(eb) = else_body {
                    collect_global_names_into(eb, names);
                }
            }
            Stmt::While(_, body)
            | Stmt::For { body, .. }
            | Stmt::ForEach { body, .. }
            | Stmt::FuncDef { body, .. } => {
                collect_global_names_into(body, names);
            }
            Stmt::Match { arms, .. } => {
                for arm in arms {
                    collect_global_names_into(&arm.body, names);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{preprocess, program_with_include, resolve_program_item_lines, ProgramItem, Talk};
    use chumsky::Parser;

    fn parse_talks_and_funcs(src: &str) -> (Vec<Talk>, Vec<(String, Vec<String>, Vec<Spanned<Stmt>>)>) {
        let pre = preprocess(src).expect("preprocess failed");
        let mut items = program_with_include().parse(&*pre.src).into_result().expect("parse failed");
        resolve_program_item_lines(&mut items, &pre.src, &pre, "main.mnt");
        let mut talks = vec![];
        let mut funcs = vec![];
        for item in items {
            match item {
                ProgramItem::Talk(t) => talks.push(t),
                ProgramItem::FuncDef { name, params, body } => funcs.push((name, params, body)),
                _ => {}
            }
        }
        (talks, funcs)
    }

    #[test]
    fn test_local_func_call_does_not_trigger_notice() {
        let src = r#"OnBoot => {
    func 挨拶する() {
        湊: こんにちは
    }
    call 挨拶する
}"#;
        let (talks, funcs) = parse_talks_and_funcs(src);
        let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
        for t in &talks {
            talk_map.entry(t.event.clone()).or_default().push(t.clone());
        }
        let talk_names: HashSet<String> = talk_map.keys().cloned().collect();
        let func_names: HashSet<String> = funcs.iter().map(|(n, _, _)| n.clone()).collect();

        let errors = Analyzer::new(talk_names, func_names).analyze(&talk_map, &funcs);

        assert!(
            !errors.iter().any(|e| e.message.contains("挨拶する") && e.message.contains("未定義")),
            "ローカル定義されたfuncへのcallがnotice誤爆している: {:?}", errors
        );
    }

#[test]
    fn test_undefined_call_still_triggers_notice() {
        // ローカル関数を許容するようになった一方で、
        // 本当に未定義のcallはちゃんと検出できることの確認（縮退防止）
        let src = r#"OnBoot => {
    call 存在しない関数
}"#;
        let (talks, funcs) = parse_talks_and_funcs(src);
        let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
        for t in &talks {
            talk_map.entry(t.event.clone()).or_default().push(t.clone());
        }
        let talk_names: HashSet<String> = talk_map.keys().cloned().collect();
        let func_names: HashSet<String> = funcs.iter().map(|(n, _, _)| n.clone()).collect();

        let errors = Analyzer::new(talk_names, func_names).analyze(&talk_map, &funcs);

        assert!(
            errors.iter().any(|e| e.message.contains("存在しない関数") && e.message.contains("未定義")),
            "本来検出すべき未定義callが見逃されている: {:?}", errors
        );
        let notice = errors.iter()
            .find(|e| e.message.contains("存在しない関数"))
            .expect("未定義callのnoticeが見つからない");
        assert_eq!(notice.line(), Some(2), "未定義callの行番号が実際のcall文の行と一致しない: {:?}", errors);
    }

    #[test]
    fn test_undefined_call_line_number_survives_dialogue_merge() {
        // トーク定義内に複数行のセリフを挟んでも、未定義callの診断のlineが
        // preprocessによる行結合の影響を受けず、元ソースの実際の行番号を指すこと。
        let src = r#"OnBoot => {
    湊: 1行目
    湊: 2行目
    call 存在しない関数
}"#;
        let (talks, funcs) = parse_talks_and_funcs(src);
        let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
        for t in &talks {
            talk_map.entry(t.event.clone()).or_default().push(t.clone());
        }
        let talk_names: HashSet<String> = talk_map.keys().cloned().collect();
        let func_names: HashSet<String> = funcs.iter().map(|(n, _, _)| n.clone()).collect();

        let errors = Analyzer::new(talk_names, func_names).analyze(&talk_map, &funcs);

        let notice = errors.iter()
            .find(|e| e.message.contains("存在しない関数") && e.message.contains("未定義"))
            .expect("本来検出すべき未定義callが見逃されている");
        assert_eq!(notice.line(), Some(4), "「call 存在しない関数」の実際の行(4行目)と一致しない: {:?}", errors);
    }

    #[test]
    fn test_break_outside_loop_line_number() {
        let src = r#"OnBoot => {
    湊: 1行目
    break
}"#;
        let (talks, funcs) = parse_talks_and_funcs(src);
        let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
        for t in &talks {
            talk_map.entry(t.event.clone()).or_default().push(t.clone());
        }
        let talk_names: HashSet<String> = talk_map.keys().cloned().collect();
        let func_names: HashSet<String> = funcs.iter().map(|(n, _, _)| n.clone()).collect();

        let errors = Analyzer::new(talk_names, func_names).analyze(&talk_map, &funcs);

        let error = errors.iter()
            .find(|e| e.message.contains("break"))
            .expect("ループ外breakのエラーが見つからない");
        assert_eq!(error.line(), Some(3), "breakの行番号が実際のbreak文の行(3行目)と一致しない: {:?}", errors);
    }

    #[test]
    fn test_continue_outside_loop_line_number_survives_dialogue_merge() {
        // セリフ行の結合を挟んでも、ループ外continueの診断のlineが
        // preprocessによる行結合の影響を受けず、元ソースの実際の行番号を指すこと。
        let src = r#"OnBoot => {
    湊: 1行目
    湊: 2行目
    湊: 3行目
    continue
}"#;
        let (talks, funcs) = parse_talks_and_funcs(src);
        let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
        for t in &talks {
            talk_map.entry(t.event.clone()).or_default().push(t.clone());
        }
        let talk_names: HashSet<String> = talk_map.keys().cloned().collect();
        let func_names: HashSet<String> = funcs.iter().map(|(n, _, _)| n.clone()).collect();

        let errors = Analyzer::new(talk_names, func_names).analyze(&talk_map, &funcs);

        let error = errors.iter()
            .find(|e| e.message.contains("continue"))
            .expect("ループ外continueのエラーが見つからない");
        assert_eq!(error.line(), Some(5), "continueの行番号が実際のcontinue文の行(5行目)と一致しない: {:?}", errors);
    }

    #[test]
    fn test_break_inside_while_no_error() {
        // ループ内のbreakはそもそもエラーにならないことの回帰確認
        // （loop_depthの管理がSpanned導入で崩れていないか）
        let src = r#"OnBoot => {
    while (true) {
        break
    }
}"#;
        let (talks, funcs) = parse_talks_and_funcs(src);
        let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
        for t in &talks {
            talk_map.entry(t.event.clone()).or_default().push(t.clone());
        }
        let talk_names: HashSet<String> = talk_map.keys().cloned().collect();
        let func_names: HashSet<String> = funcs.iter().map(|(n, _, _)| n.clone()).collect();

        let errors = Analyzer::new(talk_names, func_names).analyze(&talk_map, &funcs);

        assert!(
            !errors.iter().any(|e| e.message.contains("break")),
            "ループ内のbreakが誤ってエラーになっている: {:?}", errors
        );
    }

    #[test]
    fn test_local_func_scoped_to_its_own_talk_only() {
        // OnBootの中だけで定義したローカル関数を、
        // 別のtalk(OnClose)からcallした場合は引き続きnoticeが出ること
        // （ローカル関数のスコープがtalkを越えて漏れていないことの確認）
        let src = r#"OnBoot => {
    func 挨拶する() {
        湊: こんにちは
    }
    call 挨拶する
}
OnClose => {
    call 挨拶する
}"#;
        let (talks, funcs) = parse_talks_and_funcs(src);
        let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
        for t in &talks {
            talk_map.entry(t.event.clone()).or_default().push(t.clone());
        }
        let talk_names: HashSet<String> = talk_map.keys().cloned().collect();
        let func_names: HashSet<String> = funcs.iter().map(|(n, _, _)| n.clone()).collect();

        let errors = Analyzer::new(talk_names, func_names).analyze(&talk_map, &funcs);

        let notices_for_target: Vec<_> = errors.iter()
            .filter(|e| e.message.contains("挨拶する") && e.message.contains("未定義"))
            .collect();
        assert_eq!(
            notices_for_target.len(), 1,
            "OnCloseからのcallだけがnoticeになるべき（OnBoot内のcallは誤爆しないが、OnCloseからは見えないはず）: {:?}", errors
        );
        assert_eq!(notices_for_target[0].context.as_deref(), Some("OnClose"));
    }

    #[test]
    fn test_toplevel_func_still_recognized() {
        // トップレベルfuncの既存挙動（回帰確認）
        let src = r#"
func 共通処理() {
    湊: 共通
}
OnBoot => {
    call 共通処理
}"#;
        let (talks, funcs) = parse_talks_and_funcs(src);
        let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
        for t in &talks {
            talk_map.entry(t.event.clone()).or_default().push(t.clone());
        }
        let talk_names: HashSet<String> = talk_map.keys().cloned().collect();
        let func_names: HashSet<String> = funcs.iter().map(|(n, _, _)| n.clone()).collect();

        let errors = Analyzer::new(talk_names, func_names).analyze(&talk_map, &funcs);

        assert!(
            !errors.iter().any(|e| e.message.contains("共通処理") && e.message.contains("未定義")),
            "トップレベルfuncへのcallがnotice誤爆している: {:?}", errors
        );
    }

    #[test]
fn test_error_order_is_stable_across_multiple_talks() {
    // 複数のtalkにそれぞれ未定義callを仕込み、エラーの並びが
    // イベント名の昇順で安定することを確認する。
    // HashMap走査順に依存していた場合、この並びは実行ごとに変わりうる。
    let src = r#"
OnZzzLast => {
    call 存在しないZ
}
OnAaaFirst => {
    call 存在しないA
}
OnMmmMiddle => {
    call 存在しないM
}
"#;
    let (talks, funcs) = parse_talks_and_funcs(src);
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    for t in &talks {
        talk_map.entry(t.event.clone()).or_default().push(t.clone());
    }
    let talk_names: HashSet<String> = talk_map.keys().cloned().collect();
    let func_names: HashSet<String> = funcs.iter().map(|(n, _, _)| n.clone()).collect();

    let errors = Analyzer::new(talk_names, func_names).analyze(&talk_map, &funcs);

    let notice_events: Vec<&str> = errors.iter()
        .filter(|e| e.message.contains("未定義"))
        .map(|e| e.context.as_deref().unwrap_or(""))
        .collect();

    assert_eq!(
        notice_events,
        vec!["OnAaaFirst", "OnMmmMiddle", "OnZzzLast"],
        "エラー順序がイベント名の昇順で安定していない: {:?}", notice_events
    );
}

#[test]
fn test_error_order_stable_across_repeated_runs() {
    // 同じ入力に対してAnalyzerを複数回実行し、
    // 毎回同じ順序でエラーが出ることを確認する
    // （HashMapのランダムなハッシュシードに影響されていないことの検証）
    let src = r#"
OnZzzLast => {
    call 存在しないZ
}
OnAaaFirst => {
    call 存在しないA
}
OnMmmMiddle => {
    call 存在しないM
}
"#;
    let (talks, funcs) = parse_talks_and_funcs(src);
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    for t in &talks {
        talk_map.entry(t.event.clone()).or_default().push(t.clone());
    }

    let mut previous: Option<Vec<String>> = None;
    for _ in 0..20 {
        let talk_names: HashSet<String> = talk_map.keys().cloned().collect();
        let func_names: HashSet<String> = funcs.iter().map(|(n, _, _)| n.clone()).collect();
        let errors = Analyzer::new(talk_names, func_names).analyze(&talk_map, &funcs);
        let order: Vec<String> = errors.iter()
            .filter(|e| e.message.contains("未定義"))
            .map(|e| e.context.clone().unwrap_or_default())
            .collect();

        if let Some(ref prev) = previous {
            assert_eq!(prev, &order, "実行のたびにエラー順序が変わっている");
        }
        previous = Some(order);
    }
}

    fn analyze_src(src: &str) -> Vec<Diagnostic> {
        let (talks, funcs) = parse_talks_and_funcs(src);
        let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
        for t in &talks {
            talk_map.entry(t.event.clone()).or_default().push(t.clone());
        }
        let talk_names: HashSet<String> = talk_map.keys().cloned().collect();
        let func_names: HashSet<String> = funcs.iter().map(|(n, _, _)| n.clone()).collect();
        Analyzer::new(talk_names, func_names).analyze(&talk_map, &funcs)
    }

    fn undefined_notices(errors: &[Diagnostic]) -> Vec<&Diagnostic> {
        errors.iter().filter(|e| e.level == Level::Notice && e.message.contains("未定義")).collect()
    }

    #[test]
    fn test_dotted_call_typo_triggers_notice() {
        let src = "OnUpdate.OnDownloadBegin => {\n    湊: 開始\n}\nOnBoot => {\n    湊: 起動\n    call OnUpdate.OnTypo\n}";
        let errors = analyze_src(src);
        let n = undefined_notices(&errors);
        assert_eq!(n.len(), 1, "{:?}", errors);
        assert!(n[0].message.contains("OnUpdate.OnTypo"));
        assert_eq!(n[0].line(), Some(6));
    }

    #[test]
    fn test_dotted_call_defined_no_notice() {
        let src = "OnUpdate.OnDownloadBegin => {\n    湊: 開始\n}\nOnBoot => {\n    call OnUpdate.OnDownloadBegin\n}";
        assert!(undefined_notices(&analyze_src(src)).is_empty());
    }

    #[test]
    fn test_dynamic_dotted_call_no_notice() {
        let src = "OnUpdate.OnDownloadBegin => {\n    湊: 開始\n}\nOnChoiceSelect => {\n    call reference.0\n}";
        assert!(undefined_notices(&analyze_src(src)).is_empty());
    }

    #[test]
    fn test_dotted_call_without_group_no_notice() {
        let src = "OnBoot => {\n    call save.next\n}";
        assert!(undefined_notices(&analyze_src(src)).is_empty());
    }

    #[test]
    fn test_dotted_call_similar_prefixes_defined_no_notice() {
        let src = "OnUpdate.OnDownloadBegin => {\n    湊: a\n}\nOnUpdateOther.OnDownloadBegin => {\n    湊: b\n}\nOnBoot => {\n    call OnUpdate.OnDownloadBegin\n    call OnUpdateOther.OnDownloadBegin\n}";
        assert!(undefined_notices(&analyze_src(src)).is_empty());
    }

    #[test]
    fn test_dotted_call_prefix_includes_dot() {
        // 「OnUpdate.」が「OnUpdateOther.」に前方一致で誤って当たらないこと
        let src = "OnUpdate.OnDownloadBegin => {\n    湊: a\n}\nOnBoot => {\n    call OnUpdateOther.OnTypo\n}";
        assert!(undefined_notices(&analyze_src(src)).is_empty());
    }

    #[test]
    fn test_undefined_call_in_expression_triggers_notice() {
        // let の右辺、if の条件、セリフ中の ${...}、for の条件、関数の引数の中の呼び出しも検査する
        let src = "OnBoot => {
    let n = lenght(\"abc\")
    if (is_nmu(n)) {
        湊: ${to_uper(\"a\")}
    }
    for (let i = 0; i < lenn(\"a\"); i++) {
    }
    log(to_srt(1))
}";
        let errors = analyze_src(src);
        let n = undefined_notices(&errors);
        let found: Vec<(String, Option<u32>)> = n.iter().map(|e| (e.message.clone(), e.line())).collect();
        for (name, line) in [("lenght", 2), ("is_nmu", 3), ("to_uper", 4), ("lenn", 6), ("to_srt", 8)] {
            assert!(
                found.iter().any(|(m, l)| m.contains(&format!("\"{}\"", name)) && *l == Some(line)),
                "{} の notice（{}行目）がない: {:?}", name, line, found
            );
        }
        assert_eq!(n.len(), 5, "{:?}", found);
    }

    #[test]
    fn test_defined_names_in_expression_do_not_trigger_notice() {
        // ビルトイン・トップレベルfunc・ローカルfunc・talkを式の中で呼んでもnoticeは出ない
        let src = "func 足す(a, b) {
    return a + b
}
挨拶 => {
    湊: やあ
}
OnBoot => {
    func 引く(a, b) {
        return a - b
    }
    let n = 足す(len(\"ab\"), 引く(3, 1))
    湊: ${挨拶()}${n}
}";
        let errors = analyze_src(src);
        assert!(undefined_notices(&errors).is_empty(), "{:?}", errors);
    }

    #[test]
    fn test_statement_call_reported_once() {
        // 行頭の呼び出しは check_stmt と式の検査の両方で数えない（1件だけ）
        let src = "OnBoot => {
    挨拶すろ(lenght(\"a\"))
}";
        let errors = analyze_src(src);
        let n = undefined_notices(&errors);
        assert_eq!(n.iter().filter(|e| e.message.contains("挨拶すろ")).count(), 1, "{:?}", errors);
        assert_eq!(n.iter().filter(|e| e.message.contains("lenght")).count(), 1, "{:?}", errors);
    }

    #[test]
    fn test_undefined_call_in_talk_cond_triggers_notice() {
        // トークの if(条件式) の中の呼び出しも検査し、条件式の行で報告する
        let src = "OnBoot => {
    湊: 1行目
    湊: 2行目
}

OnRandomTalk if(is_nmu(1)) => {
    湊: a
}
OnRandomTalk
    if(len(\"a\") > 0 && 判定(1)) => {
    湊: b
}";
        let errors = analyze_src(src);
        let n = undefined_notices(&errors);
        let found: Vec<(String, Option<u32>)> = n.iter().map(|e| (e.message.clone(), e.line())).collect();
        assert_eq!(n.len(), 2, "{:?}", found);
        assert!(found.iter().any(|(m, l)| m.contains("\"is_nmu\"") && *l == Some(6)), "{:?}", found);
        assert!(found.iter().any(|(m, l)| m.contains("\"判定\"") && *l == Some(10)), "{:?}", found);
        assert!(n.iter().all(|e| e.file.as_deref() == Some("main.mnt")), "{:?}", errors);
    }

    #[test]
    fn test_defined_func_in_talk_cond_does_not_trigger_notice() {
        let src = "func 仲良し() {
    return true
}
OnRandomTalk if(仲良し() && len(\"a\") > 0) => {
    湊: a
}";
        assert!(undefined_notices(&analyze_src(src)).is_empty());
    }

    // ---- 自由変数チェック ----

    /// トップレベルのglobal文も拾ってanalyzeする。
    fn analyze_src_with_globals(src: &str) -> Vec<Diagnostic> {
        let pre = preprocess(src).expect("preprocess failed");
        let mut items = program_with_include().parse(&*pre.src).into_result().expect("parse failed");
        resolve_program_item_lines(&mut items, &pre.src, &pre, "main.mnt");
        let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
        let mut funcs = vec![];
        let mut globals = vec![];
        for item in items {
            match item {
                ProgramItem::Talk(t) => talk_map.entry(t.event.clone()).or_default().push(t),
                ProgramItem::FuncDef { name, params, body } => funcs.push((name, params, body)),
                ProgramItem::Global(path, op, expr) => globals.push((path, op, expr)),
                _ => {}
            }
        }
        let talk_names: HashSet<String> = talk_map.keys().cloned().collect();
        let func_names: HashSet<String> = funcs.iter().map(|(n, _, _)| n.clone()).collect();
        Analyzer::new(talk_names, func_names)
            .with_global_names(top_global_names(&globals))
            .analyze(&talk_map, &funcs)
    }

    fn free_var_notices(errors: &[Diagnostic]) -> Vec<&Diagnostic> {
        errors.iter()
            .filter(|e| e.level == Level::Notice
                && (e.message.contains("引数・let・global") || e.message.contains("への代入は")))
            .collect()
    }

    #[test]
    fn test_free_var_func_reads_caller_let() {
        let src = "func f() {\n    return name\n}\nOnBoot => {\n    let name = \"湊\"\n    湊: ${f()}\n}";
        let n = analyze_src_with_globals(src);
        let n = free_var_notices(&n);
        assert_eq!(n.len(), 1, "{:?}", n);
        assert!(n[0].message.contains("\"name\""));
    }

    #[test]
    fn test_free_var_params_only_no_notice() {
        let src = "func add(a, b) {\n    return a + b\n}\nOnBoot => {\n    湊: ${add(1, 2)}\n}";
        assert!(free_var_notices(&analyze_src_with_globals(src)).is_empty());
    }

    #[test]
    fn test_free_var_used_before_let() {
        let src = "OnBoot => {\n    湊: ${x}\n    let x = 1\n    湊: ${x}\n}";
        let n = analyze_src_with_globals(src);
        assert_eq!(free_var_notices(&n).len(), 1);
    }

    #[test]
    fn test_free_var_let_in_if_not_visible_after() {
        let src = "OnBoot => {\n    if (true) {\n        let x = 1\n    }\n    湊: ${x}\n}";
        assert_eq!(free_var_notices(&analyze_src_with_globals(src)).len(), 1);
    }

    #[test]
    fn test_free_var_global_read_without_keyword_no_notice() {
        let src = "global 一覧 = [1, 2]\nOnBoot => {\n    global save.回数 += 1\n    湊: ${一覧[0]} ${save.回数}\n}\nOnClose => {\n    湊: ${save.回数}\n}";
        assert!(free_var_notices(&analyze_src_with_globals(src)).is_empty());
    }

    #[test]
    fn test_free_var_ambient_names_no_notice() {
        let src = "func 時() {\n    return now.hour\n}\nOnBoot => {\n    湊: ${reference.0} ${時()} ${system.version}\n}";
        assert!(free_var_notices(&analyze_src_with_globals(src)).is_empty());
    }

    #[test]
    fn test_free_var_local_func_does_not_capture_outer_let() {
        let src = "OnBoot => {\n    let x = 1\n    func g() {\n        return x\n    }\n    湊: ${g()}\n}";
        assert_eq!(free_var_notices(&analyze_src_with_globals(src)).len(), 1);
    }

    #[test]
    fn test_free_var_compound_assign_to_let_no_notice() {
        // 行末の ; は文としては構文エラーになるので付けない
        let src = "OnBoot=>{\nlet  hoge = 1\nhoge +=2\n}";
        assert!(free_var_notices(&analyze_src_with_globals(src)).is_empty());
    }

    #[test]
    fn test_free_var_assign_without_let_reported() {
        let src = "func f() {\n    count += 1\n}\nOnBoot => {\n    f()\n}";
        let n = analyze_src_with_globals(src);
        let n = free_var_notices(&n);
        assert!(n.iter().any(|d| d.message.contains("\"count\" への代入は")), "{:?}", n);
    }

    #[test]
    fn test_free_var_reported_once_per_name() {
        let src = "func f() {\n    湊: ${x}${x}\n    return x\n}\nOnBoot => {\n    湊: ${f()}\n}";
        assert_eq!(free_var_notices(&analyze_src_with_globals(src)).len(), 1);
    }

    #[test]
    fn test_free_var_loop_variables_no_notice() {
        let src = "OnBoot => {\n    for (let i = 0; i < 3; i++) {\n        湊: ${i}\n    }\n    let items = [1, 2]
    foreach items as k, v {\n        湊: ${k}${v}\n    }\n}";
        assert!(free_var_notices(&analyze_src_with_globals(src)).is_empty());
    }

    #[test]
    fn test_free_var_for_step_is_not_a_global_declaration() {
        let src = "OnBoot => {\n    for (let i = 0; i < 3; i++) {\n    }\n}\nOnClose => {\n    湊: ${i}\n}";
        assert_eq!(free_var_notices(&analyze_src_with_globals(src)).len(), 1);
    }

    #[test]
    fn test_free_var_null_literal_no_notice() {
        let src = "func f() {
    let x = get({}, \"k\", null)
    if (is_null(x)) {
        return null
    }
    return x
}
OnBoot => {
    湊: ${f()}${null}
}";
        assert!(free_var_notices(&analyze_src_with_globals(src)).is_empty());
    }

    // ---- 未使用・書くだけの変数、到達しないコード ----

    fn by_rule<'d>(errors: &'d [Diagnostic], r: &str) -> Vec<&'d Diagnostic> {
        errors.iter().filter(|e| e.rule == Some(r)).collect()
    }

    #[test]
    fn test_unused_let_reported() {
        let d = analyze_src_with_globals("OnBoot => {\n    let x = 1\n    湊: こんにちは\n}");
        let u = by_rule(&d, rule::UNUSED_VARIABLE);
        assert_eq!(u.len(), 1, "{:?}", d);
        assert_eq!(u[0].line(), Some(2));
        assert_eq!(u[0].level, Level::Notice);
    }

    #[test]
    fn test_used_let_not_reported() {
        let d = analyze_src_with_globals("OnBoot => {\n    let x = 1\n    湊: ${x}\n}");
        assert!(by_rule(&d, rule::UNUSED_VARIABLE).is_empty(), "{:?}", d);
        assert!(by_rule(&d, rule::WRITE_ONLY_VARIABLE).is_empty(), "{:?}", d);
    }

    #[test]
    fn test_underscore_let_not_reported() {
        let d = analyze_src_with_globals("OnBoot => {\n    let _ok = f()\n    hoge()\n}\nfunc f() {\n    return 1\n}\nfunc hoge() {\n    return 1\n}");
        assert!(by_rule(&d, rule::UNUSED_VARIABLE).is_empty(), "{:?}", d);
    }

    #[test]
    fn test_write_only_let_reported() {
        let d = analyze_src_with_globals("OnBoot=>{\nlet  hoge = 1\nhoge +=2\n}");
        let w = by_rule(&d, rule::WRITE_ONLY_VARIABLE);
        assert_eq!(w.len(), 1, "{:?}", d);
        assert_eq!(w[0].line(), Some(2));
        assert!(by_rule(&d, rule::UNUSED_VARIABLE).is_empty());
    }

    #[test]
    fn test_let_read_in_nested_block_is_used() {
        let d = analyze_src_with_globals("OnBoot => {\n    let n = 0\n    while (n < 3) {\n        n += 1\n    }\n}");
        assert!(by_rule(&d, rule::WRITE_ONLY_VARIABLE).is_empty(), "{:?}", d);
        assert!(by_rule(&d, rule::UNUSED_VARIABLE).is_empty(), "{:?}", d);
    }

    #[test]
    fn test_redeclared_let_reports_first_unread() {
        let d = analyze_src_with_globals("OnBoot => {\n    let x = 1\n    let x = 2\n    湊: ${x}\n}");
        let u = by_rule(&d, rule::UNUSED_VARIABLE);
        assert_eq!(u.len(), 1, "{:?}", d);
        assert_eq!(u[0].line(), Some(2));
    }

    #[test]
    fn test_params_and_loop_vars_not_reported() {
        let d = analyze_src_with_globals("func f(a, b) {\n    return a\n}\nOnBoot => {\n    let items = [1]\n    foreach items as i, v {\n        湊: あ\n    }\n    湊: ${f(1, 2)}\n}");
        assert!(by_rule(&d, rule::UNUSED_VARIABLE).is_empty(), "{:?}", d);
    }

    #[test]
    fn test_let_used_as_dynamic_call_target_is_used() {
        let d = analyze_src_with_globals("挨拶 => {\n    湊: やあ\n}\nOnBoot => {\n    let 先 = \"挨拶\"\n    call 先\n}");
        assert!(by_rule(&d, rule::UNUSED_VARIABLE).is_empty(), "{:?}", d);
    }

    #[test]
    fn test_unused_let_in_local_func_reported_once() {
        let d = analyze_src_with_globals("OnBoot => {\n    func g() {\n        let t = 1\n        return 0\n    }\n    湊: ${g()}\n}");
        assert_eq!(by_rule(&d, rule::UNUSED_VARIABLE).len(), 1, "{:?}", d);
    }

    #[test]
    fn test_unreachable_after_return_is_warning() {
        let d = analyze_src_with_globals("func f() {\n    return 1\n    湊: 届かない\n    湊: これも\n}\nOnBoot => {\n    湊: ${f()}\n}");
        let u = by_rule(&d, rule::UNREACHABLE_CODE);
        assert_eq!(u.len(), 1, "{:?}", d);
        assert_eq!(u[0].line(), Some(3));
        assert_eq!(u[0].level, Level::Warning);
        assert!(u[0].message.contains("return"));
    }

    #[test]
    fn test_unreachable_after_break_in_loop() {
        let d = analyze_src_with_globals("OnBoot => {\n    while (true) {\n        break\n        湊: 届かない\n    }\n    湊: 届く\n}");
        let u = by_rule(&d, rule::UNREACHABLE_CODE);
        assert_eq!(u.len(), 1, "{:?}", d);
        assert_eq!(u[0].line(), Some(4));
    }

    #[test]
    fn test_return_in_if_does_not_make_rest_unreachable() {
        let d = analyze_src_with_globals("OnBoot => {\n    if (true) {\n        return\n    }\n    湊: 届く\n}");
        assert!(by_rule(&d, rule::UNREACHABLE_CODE).is_empty(), "{:?}", d);
    }
}
