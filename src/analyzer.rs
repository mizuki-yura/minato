// analyzer.rs
// ASTの静的チェック
use std::collections::{HashMap, HashSet};
use crate::parser::{Stmt, Expr, Talk};
use crate::codegen::is_builtin;
#[derive(Debug)]
pub struct AnalyzeError {
      pub level:String,
    pub event: String,
    pub message: String,
}

pub struct Analyzer<'a> {
    /// 定義済みtalk名
    talk_names: HashSet<String>,
    /// 定義済みfunc名（トップレベル＋現在の検査対象内でローカル定義されたもの）
    func_names: HashSet<String>,
    errors: Vec<AnalyzeError>,
    loop_depth: usize,
    current_context: &'a str,
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
        }
    }

pub fn analyze(
    mut self,
    talks: &'a HashMap<String, Vec<Talk>>,
    funcs: &'a [(String, Vec<String>, Vec<Stmt>)],
) -> Vec<AnalyzeError> {
    // talkを検査
    // HashMapの走査順は非決定的（実行のたびに変わりうる）なため、
    // エラーメッセージの並びを安定させるためイベント名でソートしてから走査する。
    let mut talk_names_sorted: Vec<&String> = talks.keys().collect();
    talk_names_sorted.sort();

   for event_name in talk_names_sorted {
        let talk_list = &talks[event_name];

        // @N は構文としてはパースされるが、TalkSelectorは重みを使わない
        // （全候補を均等に扱い、1周保証で選ぶ）。書いても無言で無視されるため、
        // 作者が気づけるようここで一度だけ通知する。
        // 同一イベントに複数の@Nがあってもメッセージは1件にまとめる。
        if talk_list.iter().any(|t| t.weight.is_some()) {
            self.errors.push(AnalyzeError {
                level: "warning".to_string(),
                event: event_name.clone(),
                message: "@による重み指定は現在のバージョンでは選択に影響しません（全候補が均等に、1周するまで重複しない方式で選ばれます）".to_string(),
            });
        }
        for talk in talk_list {
            self.current_context = &talk.event;
            self.loop_depth = 0;
            let local_funcs = collect_local_func_names(&talk.body);
            for name in &local_funcs { self.func_names.insert(name.clone()); }
            self.check_stmts(&talk.body);
            for name in &local_funcs { self.func_names.remove(name); }
        }
    }
    // トップレベルfuncを検査（Vec由来なので元々順序は安定）
    for (name, _params, body) in funcs {
        self.current_context = name;
        self.loop_depth = 0;
        let local_funcs = collect_local_func_names(body);
        for n in &local_funcs { self.func_names.insert(n.clone()); }
        self.check_stmts(body);
        for n in &local_funcs { self.func_names.remove(n); }
    }
    self.errors
}

    fn check_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.check_stmt(stmt);
        }
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Break => {
                if self.loop_depth == 0 {
                    self.errors.push(AnalyzeError {
                         level: "error".to_string(),
                        event: self.current_context.to_string(),
                        message: "ループの外で break を使っています".to_string(),
                    });
                }
            }
            Stmt::Continue => {
                if self.loop_depth == 0 {
                    self.errors.push(AnalyzeError {
                        level: "error".to_string(),
                        event: self.current_context.to_string(),
                        message: "ループの外で continue を使っています".to_string(),
                    });
                }
            }
           Stmt::Return(_) => {

}
            
Stmt::Call(expr) => {
    match expr {
        Expr::Var(path) if path.len() == 1 => {
            let name = &path[0];
            if !self.talk_names.contains(name) && !self.func_names.contains(name) {
                self.errors.push(AnalyzeError {
                    level: "notice".to_string(),
                    event: self.current_context.to_string(),
                    message: format!(
                        "\"{}\" は未定義です（動的callなら無視してください）",
                        name
                    ),
                });
            }
        }
        Expr::Call(name, _) => {
            if !is_builtin(name)
                && !self.talk_names.contains(name)
                && !self.func_names.contains(name)
            {
                self.errors.push(AnalyzeError {
                    level: "notice".to_string(),
                    event: self.current_context.to_string(),
                    message: format!(
                        "\"{}\" は未定義です（ビルトイン関数でも talk でも func でもありません）",
                        name
                    ),
                });
            }
        }
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
fn collect_local_func_names(stmts: &[Stmt]) -> HashSet<String> {
    let mut names = HashSet::new();
    collect_local_func_names_into(stmts, &mut names);
    names
}

fn collect_local_func_names_into(stmts: &[Stmt], names: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{preprocess, program_with_include, ProgramItem, Talk};
    use chumsky::Parser;

    fn parse_talks_and_funcs(src: &str) -> (Vec<Talk>, Vec<(String, Vec<String>, Vec<Stmt>)>) {
        let preprocessed = preprocess(src).expect("preprocess failed");
        let items = program_with_include().parse(&*preprocessed).into_result().expect("parse failed");
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
    fn test_weight_produces_warning() {
        let src = r#"
OnRandomTalk @3 => {
    湊: 重いトーク
}
OnRandomTalk => {
    湊: 重みなし
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

        let weight_warnings: Vec<_> = errors.iter()
            .filter(|e| e.level == "warning" && e.message.contains("重み"))
            .collect();
        assert_eq!(weight_warnings.len(), 1, "イベント単位で1件にまとまっていない: {:?}", errors);
        assert_eq!(weight_warnings[0].event, "OnRandomTalk");
    }


//Analyzer
    #[test]
    fn test_no_weight_produces_no_warning() {
        let src = r#"OnBoot => {
    湊: おはよう
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
            !errors.iter().any(|e| e.message.contains("重み")),
            "重み指定がないのに警告が出ている: {:?}", errors
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
        assert_eq!(notices_for_target[0].event, "OnClose");
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
        .map(|e| e.event.as_str())
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
            .map(|e| e.event.clone())
            .collect();

        if let Some(ref prev) = previous {
            assert_eq!(prev, &order, "実行のたびにエラー順序が変わっている");
        }
        previous = Some(order);
    }
}
}