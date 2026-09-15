// codegen.rs
// ASTからSAKURAスクリプトへの変換器
use crate::DEBUG_LOG;
use crate::Ordering;
use std::path::{Path, PathBuf};   // ← PathBuf 単独から変更
use encoding_rs::SHIFT_JIS;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use crate::parser::{
    AssignOp, CmpOp, Expr, Stmt, StrPart, Talk, Line, BinOp, PathSegment, MatchPattern
};
use indexmap::IndexMap;

use crate::runtime::{TalkSelector, simple_rand};
use chrono::{Datelike, Timelike, Utc, NaiveDate};

// ── ランタイム値 ──────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Value {
    Str(String),
    Number(f64),
    Bool(bool),
    Array(Vec<Value>),
    Map(IndexMap<String, Value>),
    Null,
}

impl Value {
    pub fn to_display(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Number(n) => {
                if n.fract() == 0.0 { format!("{}", *n as i64) }
                else { format!("{}", n) }
            }
            Value::Bool(b) => b.to_string(),
            Value::Array(a) => format!("[{}]", a.iter().map(|v| v.to_display()).collect::<Vec<_>>().join(", ")),
            Value::Map(_) => "[map]".to_string(),
            Value::Null => "".to_string(),
        }
    }

pub fn as_bool(&self) -> bool {
    match self {
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::Null => false,
        Value::Array(a) => !a.is_empty(),
        Value::Map(m) => !m.is_empty(),
    }
}

    pub fn as_number(&self) -> f64 {
        match self {
            Value::Number(n) => *n,
            Value::Str(s) => s.parse().unwrap_or(0.0),
            Value::Bool(b) => if *b { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }
}

// ── 実行環境 ─────────────────────────────────────────────

pub struct Env {
    pub globals: HashMap<String, Value>,
    locals: Vec<HashMap<String, Value>>,
    pub characters: HashMap<String, String>,
    pub funcs: HashMap<String, (Vec<String>, Vec<Stmt>)>,
    pub call_depth: usize,
    /// このイベント処理中にget_property/saoriを呼び出した回数。
    /// MAX_EXTERNAL_CALLS_PER_EVENTと合わせて使う（consume_external_call_budget参照）。
    /// eval_expr(&Env版、&mut self不要なアーム用)からも読み書きする必要があるため
    /// Cellで内部可変性を持たせている。Codegenごと（=Envごと）に独立しており、
    /// 旧実装のモジュール静的と違いテスト間で状態が漏れない。
    external_call_count: std::cell::Cell<usize>,
}

impl Env {
    pub fn new(characters: HashMap<String, String>) -> Self {
        Self {
            globals: HashMap::new(),
            locals: vec![HashMap::new()],
            characters,
            funcs: HashMap::new(),
            call_depth: 0,
            external_call_count: std::cell::Cell::new(0),
        }
    }

    fn push_scope(&mut self) { self.locals.push(HashMap::new()); }
    fn pop_scope(&mut self)  { self.locals.pop(); }

    fn get(&self, key: &str) -> Value {
        for scope in self.locals.iter().rev() {
            if let Some(v) = scope.get(key) { return v.clone(); }
        }
        self.globals.get(key).cloned().unwrap_or(Value::Null)
    }

    #[allow(dead_code)]
    pub fn get_path(&self, path: &[PathSegment]) -> Value {
        match path {
            [] => Value::Null,
            [PathSegment::Key(key)] => self.get(key),
            [PathSegment::Key(head), rest @ ..] => {
                let root = self.get(head);
                get_nested(root, rest, self)
            }
            [PathSegment::Index(_), ..] => Value::Null,
        }
    }

    pub fn get_path_str(&self, path: &[String]) -> Value {
        match path {
            [] => Value::Null,
            [key] => self.get(key),
            [head, rest @ ..] => {
                let root = self.get(head);
                get_nested_str(root, rest)
            }
        }
    }




    // codegen.rs — Env の impl ブロック内に追加
// set_var（単一キー）のドット付きパス版。
// head がどこかのローカルスコープにあればそのスコープ内で更新し、
// なければ従来どおりglobalsを更新する。
// ※ global文（set_path）とは別物。global文は常にglobals直行のまま変更しない。



// codegen.rs — impl Env 内

/// 解決済みの文字列キー列でglobalsに書き込む。
/// PathSegment::Indexの評価はCodegen::resolve_path_segmentsが
/// eval_expr_fullで事前に済ませている想定。
pub fn set_path_resolved(&mut self, path: &[String], op: &AssignOp, val: Value) {
    match path {
        [] => {}
        [key] => {
            let new_val = apply_op(self.get(key), op, val);
            self.globals.insert(key.clone(), new_val);
        }
        [head, rest @ ..] => {
            let root = self.globals.remove(head).unwrap_or(Value::Null);
            let updated = set_nested_str(root, rest, op, val);
            self.globals.insert(head.clone(), updated);
        }
    }
}

/// 旧API。PathSegment::Indexの評価はeval_expr(自由関数、builtinのみ)しか使えず、
/// choose/saori/log/talk_exists/days_since/formatをインデックス式内で使うと
/// 常にNullになる制限が残る。Codegen経由の実行では使われず、
/// Envを直接叩くテスト（set_path_testsモジュール等）のために残してある。
 #[allow(dead_code)] 
pub fn set_path(&mut self, path: &[PathSegment], op: &AssignOp, val: Value) {
    let resolved: Vec<String> = path.iter().map(|seg| match seg {
        PathSegment::Key(k) => k.clone(),
        PathSegment::Index(expr) => eval_expr(expr, &*self).to_display(),
    }).collect();
    self.set_path_resolved(&resolved, op, val);
}

/// 解決済みの文字列キー列で代入する（globalキーワード無しのドット付き代入用）。
/// headがどこかのローカルスコープにあればそのスコープ内を更新し、なければglobalsを更新する。
pub fn set_var_path_resolved(&mut self, path: &[String], op: &AssignOp, val: Value) {
    match path {
        [] => {}
        [key] => {
            self.set_var(key, op, val);
        }
        [head, rest @ ..] => {
            for i in (0..self.locals.len()).rev() {
                if self.locals[i].contains_key(head) {
                    let root = self.locals[i].remove(head).unwrap_or(Value::Null);
                    let updated = set_nested_str(root, rest, op, val);
                    self.locals[i].insert(head.clone(), updated);
                    return;
                }
            }
            let root = self.globals.remove(head).unwrap_or(Value::Null);
            let updated = set_nested_str(root, rest, op, val);
            self.globals.insert(head.clone(), updated);
        }
    }
}


/// 旧API。set_pathと同じ理由でEnv直接テスト用に残してある。

    #[allow(dead_code)]
pub fn set_var_path(&mut self, path: &[PathSegment], op: &AssignOp, val: Value) {
    let resolved: Vec<String> = path.iter().map(|seg| match seg {
        PathSegment::Key(k) => k.clone(),
        PathSegment::Index(expr) => eval_expr(expr, &*self).to_display(),
    }).collect();
    self.set_var_path_resolved(&resolved, op, val);
}



    pub fn set_local(&mut self, key: &str, val: Value) {
        if let Some(scope) = self.locals.last_mut() {
            scope.insert(key.to_string(), val);
        }
    }

    pub fn set_var(&mut self, key: &str, op: &AssignOp, val: Value) {
        for scope in self.locals.iter_mut().rev() {
            if scope.contains_key(key) {
                let current = scope.get(key).cloned().unwrap_or(Value::Null);
                scope.insert(key.to_string(), apply_op(current, op, val));
                return;
            }
        }
        let current = self.globals.get(key).cloned().unwrap_or(Value::Null);
        self.globals.insert(key.to_string(), apply_op(current, op, val));
    }

        /// パニックなどで正常に巻き戻らなかった実行時状態を初期状態に戻す。
    /// globals / characters / funcs は保持する（セーブ対象・定義済みのため）。
    ///
    /// requestはcatch_unwindでパニックを握り潰すため、
    /// gen_event中に落ちるとpush_scopeしたローカルスコープと
    /// インクリメント済みのcall_depthが積まれたまま次のイベントに持ち越される。
    /// 死んだローカル変数がglobalsをシャドウし続けたり、
    /// 数回のパニックでcall_depthが100を超えてcallが一切効かなくなる。
    pub fn reset_runtime_state(&mut self) {
        self.locals.clear();
        self.locals.push(HashMap::new());
        self.call_depth = 0;
        self.external_call_count.set(0);
    }
}

// ── ネストアクセス ────────────────────────────────────────

fn get_nested(val: Value, path: &[PathSegment], env: &Env) -> Value {
    match (val, path) {
        (v, []) => v,
        (Value::Map(mut m), [PathSegment::Key(key), rest @ ..]) => {
            let child = m.shift_remove(key).unwrap_or(Value::Null);
            get_nested(child, rest, env)
        }
        (Value::Map(mut m), [PathSegment::Index(idx), rest @ ..]) => {
            let key = eval_expr(idx, env).to_display();
            let child = m.shift_remove(&key).unwrap_or(Value::Null);
            get_nested(child, rest, env)
        }
        (Value::Array(arr), [PathSegment::Index(idx), rest @ ..]) => {
            let i = eval_expr(idx, env).as_number() as usize;
            let child = arr.get(i).cloned().unwrap_or(Value::Null);
            get_nested(child, rest, env)
        }
        _ => Value::Null,
    }
}
fn get_nested_str(val: Value, path: &[String]) -> Value {
    match (val, path) {
        (v, []) => v,
        (Value::Map(mut m), [key, rest @ ..]) => {
            let child = m.shift_remove(key).unwrap_or(Value::Null);
            get_nested_str(child, rest)
        }
        // set_nested_str は数値キーで配列を辿れるが、
        // こちら（読み取り側）は元々Mapしか扱っておらず、
        // ${arr.0} が常にNullになる非対称があった。
        (Value::Array(arr), [key, rest @ ..]) => {
            match key.parse::<usize>() {
                Ok(i) => {
                    let child = arr.get(i).cloned().unwrap_or(Value::Null);
                    get_nested_str(child, rest)
                }
                Err(_) => Value::Null,
            }
        }
        _ => Value::Null,
    }
}

fn set_nested_str(val: Value, path: &[String], op: &AssignOp, new_val: Value) -> Value {
    match (val, path) {
        (v, []) => apply_op(v, op, new_val),
        (Value::Map(mut m), [key, rest @ ..]) => {
            let child = m.shift_remove(key).unwrap_or(Value::Null);
            m.insert(key.clone(), set_nested_str(child, rest, op, new_val));
            Value::Map(m)
        }
        (Value::Null, [key, rest @ ..]) => {
            let mut m = IndexMap::new();
            m.insert(key.clone(), set_nested_str(Value::Null, rest, op, new_val));
            Value::Map(m)
        }
        (Value::Array(mut arr), [key, rest @ ..]) => {
            if let Ok(i) = key.parse::<usize>() {
                if i < arr.len() {
                    arr[i] = set_nested_str(arr[i].clone(), rest, op, new_val);
                }
            }
            Value::Array(arr)
        }
        (v, _) => v,
    }
}

// ── kana_key ─────────────────────────────────────────────

fn kana_key(val: &Value) -> String {
    val.to_display().chars().map(normalize_kana).collect()
}

fn normalize_kana(c: char) -> char {
    let c = if ('\u{30A1}'..='\u{30F6}').contains(&c) {
        char::from_u32(c as u32 - 0x60).unwrap_or(c)
    } else { c };
    let c = match c {
        'が' => 'か', 'ぎ' => 'き', 'ぐ' => 'く', 'げ' => 'け', 'ご' => 'こ',
        'ざ' => 'さ', 'じ' => 'し', 'ず' => 'す', 'ぜ' => 'せ', 'ぞ' => 'そ',
        'だ' => 'た', 'ぢ' => 'ち', 'づ' => 'つ', 'で' => 'て', 'ど' => 'と',
        'ば' => 'は', 'び' => 'ひ', 'ぶ' => 'ふ', 'べ' => 'へ', 'ぼ' => 'ほ',
        'ぱ' => 'は', 'ぴ' => 'ひ', 'ぷ' => 'ふ', 'ぺ' => 'へ', 'ぽ' => 'ほ',
        _ => c,
    };
    match c {
        'ぁ' => 'あ', 'ぃ' => 'い', 'ぅ' => 'う', 'ぇ' => 'え', 'ぉ' => 'お',
        'っ' => 'つ', 'ゃ' => 'や', 'ゅ' => 'ゆ', 'ょ' => 'よ',
        'ゎ' => 'わ', 'ー' => 'あ',
        _ => c,
    }
}

// ── 演算子適用 ────────────────────────────────────────────

fn apply_op(current: Value, op: &AssignOp, val: Value) -> Value {
    match op {
        AssignOp::Set => val,
        AssignOp::Add => match (&current, &val) {
            (Value::Str(a), _) => Value::Str(format!("{}{}", a, val.to_display())),
            (_, Value::Str(b)) => Value::Str(format!("{}{}", current.to_display(), b)),
            _ => Value::Number(current.as_number() + val.as_number()),
        },
        AssignOp::Sub => Value::Number(current.as_number() - val.as_number()),
        AssignOp::Mul => Value::Number(current.as_number() * val.as_number()),
        AssignOp::Div => {
            let r = val.as_number();
            if r == 0.0 { Value::Null } else { Value::Number(current.as_number() / r) }
        }
        AssignOp::Mod => {
    let r = val.as_number();
    if r == 0.0 { Value::Null } else { Value::Number(current.as_number() % r) }
}
        AssignOp::SetIfNull => {
            if matches!(current, Value::Null) { val } else { current }
        }
    }
}

// ── 値の等価判定 ──────────────────────────────────────────
// to_display()による文字列比較をベースにしつつ、Map/Arrayだけは
// 中身を再帰的に比較する。Map.to_display()は中身に関わらず常に
// "[map]"を返すため、素朴なto_display()比較ではMap同士が常に
// 一致してしまう不具合があった（Arrayも入れ子にMapを含む場合は同様）。
// 一方、reference.0 == 1 のような文字列⇄数値の緩い比較は
// 既存スクリプトが広く依存しているため、Str/Number/Bool/Nullは
// 従来通りto_display()比較のままにする。
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Map(ma), Value::Map(mb)) => map_values_equal(ma, mb),
        (Value::Map(_), _) | (_, Value::Map(_)) => false,
        (Value::Array(aa), Value::Array(bb)) => {
            aa.len() == bb.len()
                && aa.iter().zip(bb.iter()).all(|(x, y)| values_equal(x, y))
        }
        (Value::Array(_), _) | (_, Value::Array(_)) => false,
        _ => a.to_display() == b.to_display(),
    }
}

fn map_values_equal(a: &IndexMap<String, Value>, b: &IndexMap<String, Value>) -> bool {
    a.len() == b.len()
        && a.iter().all(|(k, v)| b.get(k).map_or(false, |bv| values_equal(v, bv)))
}

// DEBUG_LOG の近くに追加
fn append_script_log(msg: &str, dir: &std::path::Path) {
    if !DEBUG_LOG.load(Ordering::Relaxed) { return; }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("minato_debug.log"))
    {
        let _ = writeln!(f, "{}", msg);
    }
}


const FILE_READ_LIMIT: u64 = 1024 * 1024;

const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul",
    "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9",
    "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// パス解決の失敗理由。
/// Deniedは「台本の書き方が間違っている」= errorレベル。
/// condで除外されたtalk由来でも作者に伝わるよう、filter_aliveで捨てられない。
/// Ioは「実行時にたまたま無い/読めない」= warningレベル。
enum PathReject {
    Denied(String),
    Io(String),
}

/// SSPが渡すのは (myghost)\ghost\master\ なので、末尾がそれと一致するときだけ
/// 2つ上をホームとみなす。テストのようにその構成でない場合はghost_dir自身に
/// フォールバックする（うっかりテンポラリディレクトリ全体がサンドボックスに
/// なるのを防ぐため、無条件にparent().parent()はしない）。
fn compute_home_dir(ghost_dir: &Path) -> PathBuf {
    let is_master = ghost_dir.file_name()
        .map(|n| n.eq_ignore_ascii_case("master")).unwrap_or(false);
    let parent_is_ghost = ghost_dir.parent()
        .and_then(|p| p.file_name())
        .map(|n| n.eq_ignore_ascii_case("ghost")).unwrap_or(false);
    if is_master && parent_is_ghost {
        ghost_dir.parent().and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| ghost_dir.to_path_buf())
   } else {
        append_log!(format!(
            "home_root fallback: ghost/master構成ではないため{:?}自身をホームとして扱います",
            ghost_dir
        ));
        ghost_dir.to_path_buf()
    }
}

/// 台本が書いたパス文字列を、ホームからの相対セグメント列に分解する。
/// canonicalizeは書き込み先に使えない（まだ存在しないため）ので、
/// 脱出の可能性がある形は全てここで文字列のまま弾いておく。
fn validate_rel_path(p: &str) -> Result<Vec<String>, String> {
    if p.trim().is_empty() {
        return Err("ファイルパスが空です".to_string());
    }
    if p.contains(':') {
        return Err(format!(
            "ドライブ指定や絶対パスは使えません（ゴーストのホームからの相対パスで書いてください）: 「{}」", p
        ));
    }
    if p.starts_with('/') || p.starts_with('\\') {
        return Err(format!(
            "絶対パスは使えません（ゴーストのホームからの相対パスで書いてください）: 「{}」", p
        ));
    }
    let mut segs: Vec<String> = Vec::new();
    for seg in p.split(|c| c == '/' || c == '\\') {
        if seg.is_empty() || seg == "." { continue; }
        if seg == ".." {
            return Err(format!("「..」を含むパスは使えません: 「{}」", p));
        }
        // Windowsは末尾の空白・ピリオドを黙って落とすため、
        // 「a.txt.」と「a.txt」が同じファイルを指してしまう。禁止判定をすり抜ける。
        if seg.ends_with(' ') || seg.ends_with('.') {
            return Err(format!("末尾が空白またはピリオドのパスは使えません: 「{}」", seg));
        }
        let stem = seg.split('.').next().unwrap_or(seg).to_ascii_lowercase();
        if WINDOWS_RESERVED.contains(&stem.as_str()) {
            return Err(format!("Windowsの予約名は使えません: 「{}」", seg));
        }
        segs.push(seg.to_string());
    }
    if segs.is_empty() {
        return Err(format!("ファイルパスが空です: 「{}」", p));
    }
    Ok(segs)
}

/// 書き込み可能か判定する。segsはホームからの相対セグメント列。
/// 許可リスト方式（ghost/master配下のみ）にしてあるので、
/// SSPが将来ホーム直下に何を置いても穴にならない。
fn check_writable(segs: &[String]) -> Result<(), String> {
    let lower: Vec<String> = segs.iter().map(|s| s.to_lowercase()).collect();
    if lower.len() < 3 || lower[0] != "ghost" || lower[1] != "master" {
        return Err(format!(
            "書き込めるのは ghost/master 配下だけです: 「{}」", segs.join("/")
        ));
    }
    let rest = &lower[2..];
    let last = rest.last().unwrap();

    if rest[0] == "talks" {
        return Err("talks配下（台本）には書き込めません".to_string());
    }
    if last.starts_with("minato_") {
        return Err(format!("「{}」は湊自身が使うファイル名です", segs.last().unwrap()));
    }
    if last.ends_with(".dll") {
        return Err("DLLには書き込めません".to_string());
    }
    if rest.len() == 1
        && (rest[0] == "config.toml"
            || rest[0] == "descript.txt"
            || rest[0].starts_with("save.json"))
    {
        return Err(format!("「{}」には書き込めません", segs.last().unwrap()));
    }
    Ok(())
}

        /// 末端がリンク（シンボリックリンク／ジャンクション）でないことを確認する。
/// Windowsではreparse pointもここで弾かれる。
fn ensure_not_link(path: &Path, shown: &str) -> Result<(), PathReject> {
    match std::fs::symlink_metadata(path) {
        Ok(md) if md.file_type().is_symlink() => Err(PathReject::Denied(
            format!("リンクは指定できません: 「{}」", shown)
        )),
        _ => Ok(()),
    }
}

fn encode_text(s: &str, enc: &str) -> Result<Vec<u8>, String> {
    match enc {
        "" | "utf8" | "utf-8" => Ok(s.as_bytes().to_vec()),
        "sjis" | "shift_jis" | "shift-jis" => {
            let (bytes, _, had_errors) = SHIFT_JIS.encode(s);
            if had_errors {
                Err("Shift_JISに変換できない文字が含まれています".to_string())
            } else {
                Ok(bytes.into_owned())
            }
        }
        other => Err(format!("未対応の文字コードです: 「{}」（utf8 か sjis）", other)),
    }
}

fn decode_text(bytes: &[u8], enc: &str) -> Result<String, String> {
    match enc {
        "" | "utf8" | "utf-8" => String::from_utf8(bytes.to_vec()).map_err(|_| {
            "UTF-8として読めませんでした（Shift_JISなら第2引数に'sjis'を指定してください）".to_string()
        }),
        "sjis" | "shift_jis" | "shift-jis" => {
            let (s, _, had_errors) = SHIFT_JIS.decode(bytes);
            if had_errors {
                Err("Shift_JISとして読めませんでした".to_string())
            } else {
                Ok(s.into_owned())
            }
        }
        other => Err(format!("未対応の文字コードです: 「{}」（utf8 か sjis）", other)),
    }
}


pub const BUILTIN_NAMES: &[&str] = &[
    "floor", "ceil", "round", "trunc", "rand", "len", "abs",
    "min", "max", "clamp",
    "sin", "cos", "tan", "asin", "acos", "atan2",
    "sqrt", "PI", "to_rad", "to_deg", "to_hex",
    "contains", "starts_with", "ends_with",
    "replace", "split", "join", "trim", "substr",
    "regex_match", "regex_find", "regex_captures",
    "regex_replace", "regex_split",
  "to_str" , "to_num" , "to_lower" , "to_upper" , "chr" , "index_of",
    "first", "last", "push", "pop", "slice",
    "has_key", "keys", "values", "delete",
    "count", "sort", "reverse", "unique",
    "get_property", "choose", "days_since", "saori", "log" , "is_null","format","talk_exists","get", "days_between", 
        "file_read", "file_write", "file_append", "file_move",
];

pub fn is_builtin(name: &str) -> bool {
    BUILTIN_NAMES.contains(&name)
}
// ── ビルトイン関数 ────────────────────────────────────────

fn call_builtin(name: &str, vals: Vec<Value>, env: &Env) -> Value {
    match name {
        "floor" => Value::Number(vals.get(0).map(|v| v.as_number().floor()).unwrap_or(0.0)),
        "ceil"  => Value::Number(vals.get(0).map(|v| v.as_number().ceil()).unwrap_or(0.0)),
        "round" => Value::Number(vals.get(0).map(|v| v.as_number().round()).unwrap_or(0.0)),
        "trunc" => Value::Number(vals.get(0).map(|v| v.as_number().trunc()).unwrap_or(0.0)),
        "rand"  => Value::Number(simple_rand() as f64),
        "len" => match vals.get(0) {
            Some(Value::Str(s))   => Value::Number(s.chars().count() as f64),
            Some(Value::Array(a)) => Value::Number(a.len() as f64),
            Some(Value::Map(m))   => Value::Number(m.len() as f64),
            _                     => Value::Number(0.0),
        },
        "abs" => Value::Number(vals.get(0).map(|v| v.as_number().abs()).unwrap_or(0.0)),
        "min" => match (vals.get(0), vals.get(1)) {
            (Some(a), Some(b)) => Value::Number(a.as_number().min(b.as_number())),
            (Some(a), None)    => Value::Number(a.as_number()),
            _                  => Value::Null,
        },
        "max" => match (vals.get(0), vals.get(1)) {
            (Some(a), Some(b)) => Value::Number(a.as_number().max(b.as_number())),
            (Some(a), None)    => Value::Number(a.as_number()),
            _                  => Value::Null,
        },
        "clamp" => match (vals.get(0), vals.get(1), vals.get(2)) {
            (Some(v), Some(lo), Some(hi)) =>
                Value::Number(v.as_number().clamp(lo.as_number(), hi.as_number())),
            _ => Value::Null,
        },
        "sin"   => Value::Number(vals.get(0).map(|v| v.as_number().sin()).unwrap_or(0.0)),
        "cos"   => Value::Number(vals.get(0).map(|v| v.as_number().cos()).unwrap_or(0.0)),
        "tan"   => Value::Number(vals.get(0).map(|v| v.as_number().tan()).unwrap_or(0.0)),
        "asin"  => Value::Number(vals.get(0).map(|v| v.as_number().asin()).unwrap_or(0.0)),
        "acos"  => Value::Number(vals.get(0).map(|v| v.as_number().acos()).unwrap_or(0.0)),
        "atan2" => match (vals.get(0), vals.get(1)) {
            (Some(y), Some(x)) => Value::Number(y.as_number().atan2(x.as_number())),
            _ => Value::Number(0.0),
        },
        "sqrt"   => Value::Number(vals.get(0).map(|v| v.as_number().sqrt()).unwrap_or(0.0)),
        "PI"     => Value::Number(std::f64::consts::PI),
        "to_rad" => Value::Number(vals.get(0).map(|v| v.as_number().to_radians()).unwrap_or(0.0)),
        "to_deg" => Value::Number(vals.get(0).map(|v| v.as_number().to_degrees()).unwrap_or(0.0)),
        "to_hex" => {
            let n = vals.get(0).map(|v| v.as_number() as i64).unwrap_or(0);
            // as_number()はf64なので、辞書スクリプトが1e20のような巨大な値を
            // 渡すとusizeへのsaturatingキャストでusize::MAXになりうる。
            // format!のwidthに巨大な値を渡すとOOM即abortに直結するため
            // FORMAT_MAX_WIDTHでクランプする。
            let digits = vals.get(1).map(|v| (v.as_number() as usize).min(FORMAT_MAX_WIDTH)).unwrap_or(0);
            if digits > 0 { Value::Str(format!("{:0>width$x}", n, width = digits)) }
            else          { Value::Str(format!("{:x}", n)) }
        }
        "contains" => match (vals.get(0), vals.get(1)) {
            (Some(v), Some(sub)) => Value::Bool(v.to_display().contains(sub.to_display().as_str())),
            _ => Value::Bool(false),
        },
        "starts_with" => match (vals.get(0), vals.get(1)) {
            (Some(v), Some(p)) => Value::Bool(v.to_display().starts_with(p.to_display().as_str())),
            _ => Value::Bool(false),
        },
        "ends_with" => match (vals.get(0), vals.get(1)) {
            (Some(v), Some(s)) => Value::Bool(v.to_display().ends_with(s.to_display().as_str())),
            _ => Value::Bool(false),
        },
        "replace" => match (vals.get(0), vals.get(1), vals.get(2)) {
            (Some(s), Some(from), Some(to)) =>
                Value::Str(s.to_display().replace(from.to_display().as_str(), to.to_display().as_str())),
            _ => Value::Null,
        },
        "split" => match (vals.get(0), vals.get(1)) {
            (Some(s), Some(sep)) => {
                let sep_s = sep.to_display();
                let parts: Vec<Value> = if sep_s.is_empty() {
                    s.to_display().chars().map(|c| Value::Str(c.to_string())).collect()
                } else {
                    s.to_display().split(sep_s.as_str()).map(|p| Value::Str(p.to_string())).collect()
                };
                Value::Array(parts)
            }
            _ => Value::Array(vec![]),
        },
        "regex_match" => match (vals.get(0), vals.get(1)) {
            (Some(s), Some(pat)) => match Regex::new(&pat.to_display()) {
                Ok(re) => Value::Bool(re.is_match(&s.to_display())),
                Err(_) => Value::Bool(false),
            },
            _ => Value::Bool(false),
        },
        "regex_find" => match (vals.get(0), vals.get(1)) {
            (Some(s), Some(pat)) => match Regex::new(&pat.to_display()) {
                Ok(re) => re.find(&s.to_display()).map(|m| Value::Str(m.as_str().to_string())).unwrap_or(Value::Null),
                Err(_) => Value::Null,
            },
            _ => Value::Null,
        },
        "regex_captures" => match (vals.get(0), vals.get(1)) {
            (Some(s), Some(pat)) => match Regex::new(&pat.to_display()) {
                Ok(re) => match re.captures(&s.to_display()) {
                    Some(caps) => Value::Array(caps.iter().map(|m| match m {
                        Some(m) => Value::Str(m.as_str().to_string()),
                        None => Value::Null,
                    }).collect()),
                    None => Value::Null,
                },
                Err(_) => Value::Null,
            },
            _ => Value::Null,
        },
        "regex_replace" => match (vals.get(0), vals.get(1), vals.get(2)) {
            (Some(s), Some(pat), Some(rep)) => match Regex::new(&pat.to_display()) {
                Ok(re) => Value::Str(re.replace_all(&s.to_display(), rep.to_display().as_str()).to_string()),
                Err(_) => vals.get(0).cloned().unwrap_or(Value::Null),
            },
            _ => Value::Null,
        },
        "regex_split" => match (vals.get(0), vals.get(1)) {
            (Some(s), Some(pat)) => match Regex::new(&pat.to_display()) {
                Ok(re) => Value::Array(re.split(&s.to_display()).map(|p| Value::Str(p.to_string())).collect()),
                Err(_) => Value::Array(vec![vals.get(0).cloned().unwrap_or(Value::Null)]),
            },
            _ => Value::Array(vec![]),
        },
        "join" => match vals.get(0) {
            Some(Value::Array(arr)) => {
                let sep = vals.get(1).map(|v| v.to_display()).unwrap_or_default();
                Value::Str(arr.iter().map(|v| v.to_display()).collect::<Vec<_>>().join(&sep))
            }
            _ => Value::Str(String::new()),
        },
        "trim"   => Value::Str(vals.get(0).map(|v| v.to_display().trim().to_string()).unwrap_or_default()),
        "to_str" => Value::Str(vals.get(0).map(|v| v.to_display()).unwrap_or_default()),
        "to_num" => Value::Number(vals.get(0).map(|v| v.as_number()).unwrap_or(0.0)),
        "to_lower" => Value::Str(vals.get(0).map(|v| v.to_display().to_lowercase()).unwrap_or_default()),
"to_upper" => Value::Str(vals.get(0).map(|v| v.to_display().to_uppercase()).unwrap_or_default()),
        "chr" => {
            let n = vals.get(0).map(|v| v.as_number() as u32).unwrap_or(0);
            match char::from_u32(n) {
                Some(c) => Value::Str(c.to_string()),
                None    => Value::Str(String::new()),
            }
        }
 "index_of" => match (vals.get(0), vals.get(1)) {
    (Some(Value::Array(arr)), Some(target)) => {
        Value::Number(arr.iter().position(|v| values_equal(v, target)).map(|i| i as f64).unwrap_or(-1.0))
    }
    (Some(s), Some(sub)) => {
        let s = s.to_display(); let sub = sub.to_display();
        Value::Number(s.find(sub.as_str()).map(|bp| s[..bp].chars().count() as f64).unwrap_or(-1.0))
    }
    _ => Value::Number(-1.0),
},
        "first" => match vals.get(0) {
            Some(Value::Array(arr)) => arr.first().cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        },
        "last" => match vals.get(0) {
            Some(Value::Array(arr)) => arr.last().cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        },
        "push" => match vals.get(0) {
            Some(Value::Array(arr)) => {
                let mut new = arr.clone();
                if let Some(v) = vals.get(1) { new.push(v.clone()); }
                Value::Array(new)
            }
            _ => Value::Null,
        },
        "pop" => match vals.get(0) {
            Some(Value::Array(arr)) => { let mut new = arr.clone(); new.pop(); Value::Array(new) }
            _ => Value::Null,
        },
        "slice" => match vals.get(0) {
            Some(Value::Array(arr)) => {
                let len = arr.len() as i64;
                let start = vals.get(1).map(|v| v.as_number() as i64).unwrap_or(0);
                let start = if start < 0 { (len + start).max(0) as usize } else { start.min(len) as usize };
                let end = vals.get(2).map(|v| {
                    let e = v.as_number() as i64;
                    if e < 0 { (len + e).max(0) as usize } else { e.min(len) as usize }
                }).unwrap_or(arr.len());
                Value::Array(arr[start..end.max(start)].to_vec())
            }
            _ => Value::Array(vec![]),
        },
        "has_key" => match (vals.get(0), vals.get(1)) {
            (Some(Value::Map(map)), Some(key)) => Value::Bool(map.contains_key(&key.to_display())),
            _ => Value::Bool(false),
        },
        "keys" => match vals.get(0) {
            Some(Value::Map(map)) => Value::Array(map.keys().map(|k| Value::Str(k.clone())).collect()),
            _ => Value::Array(vec![]),
        },
        "values" => match vals.get(0) {
            Some(Value::Map(map)) => Value::Array(map.values().cloned().collect()),
            _ => Value::Array(vec![]),
        },
        "delete" => match (vals.get(0), vals.get(1)) {
            (Some(Value::Map(map)), Some(key)) => {
                let mut new = map.clone();
                new.shift_remove(&key.to_display());
                Value::Map(new)
            }
            _ => Value::Null,
        },
        "count" => match (vals.get(0), vals.get(1)) {
    (Some(Value::Str(s)), Some(sub)) => {
        let sub = sub.to_display();
        if sub.is_empty() { Value::Number(0.0) }
        else { Value::Number(s.matches(sub.as_str()).count() as f64) }
    }
    (Some(Value::Array(arr)), Some(target)) => {
        Value::Number(arr.iter().filter(|v| values_equal(v, target)).count() as f64)
    }
    _ => Value::Number(0.0),
},
        "sort" => match vals.get(0) {
            Some(Value::Array(arr)) => {
                let mode = vals.get(1).map(|v| v.to_display()).unwrap_or_default();
                let mut new = arr.clone();
                match mode.as_str() {
                    "kana" => new.sort_by(|a, b| kana_key(a).cmp(&kana_key(b))),
                    "desc" => new.sort_by(|a, b| match (a, b) {
                        (Value::Number(x), Value::Number(y)) => y.partial_cmp(x).unwrap_or(std::cmp::Ordering::Equal),
                        _ => b.to_display().cmp(&a.to_display()),
                    }),
                    _ => new.sort_by(|a, b| match (a, b) {
                        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
                        _ => a.to_display().cmp(&b.to_display()),
                    }),
                }
                Value::Array(new)
            }
            _ => Value::Array(vec![]),
        },
        "reverse" => match vals.get(0) {
            Some(Value::Array(arr)) => { let mut new = arr.clone(); new.reverse(); Value::Array(new) }
            _ => Value::Array(vec![]),
        },
   "unique" => match vals.get(0) {
    Some(Value::Array(arr)) => {
        let mut result: Vec<Value> = Vec::new();
        for v in arr {
            if !result.iter().any(|existing| values_equal(existing, v)) {
                result.push(v.clone());
            }
        }
        Value::Array(result)
    }
    _ => Value::Array(vec![]),
},
        "get_property" => {
            let name = vals.get(0).map(|v| v.to_display()).unwrap_or_default();
            if name.is_empty() { Value::Str(String::new()) }
            else if !consume_external_call_budget(env) {
                // ループ内でget_property/saoriを繰り返し呼ぶ台本が、
                // STATEのMutexを保持したまま実質無制限にSSP全体を
                // ブロックしないための上限（詳細はMAX_EXTERNAL_CALLS_PER_EVENT参照）。
                append_log!(format!(
                    "get_property: 1イベントあたりの外部呼び出し上限（{}回）に達したため無視しました: {}",
                    MAX_EXTERNAL_CALLS_PER_EVENT, name
                ));
                Value::Str(String::new())
            }
            else { Value::Str(crate::sstp::get_property(&name)) }
        },
        "substr" => match vals.get(0) {
            Some(s) => {
                let s = s.to_display();
                let chars: Vec<char> = s.chars().collect();
                let len = chars.len() as i64;
                let start = vals.get(1).map(|v| v.as_number() as i64).unwrap_or(0);
                let start = if start < 0 { (len + start).max(0) as usize } else { start.min(len) as usize };
                let count = vals.get(2).map(|v| v.as_number() as usize).unwrap_or(chars.len() - start);
                Value::Str(chars.iter().skip(start).take(count).collect())
            }
            _ => Value::Str(String::new()),
        },
        "choose" => {
            let cond = vals.get(0).map(|v| v.as_bool()).unwrap_or(false);
            if cond { vals.get(1).cloned().unwrap_or(Value::Null) }
            else    { vals.get(2).cloned().unwrap_or(Value::Null) }
        },

        "get" => {
    // get(collection, index_or_key, default?) — 範囲外/未存在キーでも警告を出さず、
    // 見つからなければdefault（省略時はNull）を返す
    match (vals.get(0), vals.get(1)) {
        (Some(Value::Array(arr)), Some(idx)) => {
            let i = idx.as_number();
            if i < 0.0 {
                vals.get(2).cloned().unwrap_or(Value::Null)
            } else {
                arr.get(i as usize).cloned().unwrap_or_else(|| vals.get(2).cloned().unwrap_or(Value::Null))
            }
        }
        (Some(Value::Map(map)), Some(key)) => {
            map.get(&key.to_display()).cloned().unwrap_or_else(|| vals.get(2).cloned().unwrap_or(Value::Null))
        }
        _ => vals.get(2).cloned().unwrap_or(Value::Null),
    }
},

        _ => Value::Null,

        
    }
}

// ── 式の評価（&Env版、&mut self不要なアーム用）────────────

pub fn eval_expr(expr: &Expr, env: &Env) -> Value {
    match expr {
        Expr::Str(s)    => Value::Str(s.clone()),
        Expr::Number(n) => Value::Number(*n),
        Expr::Bool(b)   => Value::Bool(*b),
        Expr::Var(path) => env.get_path_str(path),
        Expr::InterpolatedStr(parts) => {
            Value::Str(parts.iter().map(|p| match p {
                StrPart::Lit(s)    => s.clone(),
                StrPart::Var(path) => env.get_path_str(path).to_display(),
                StrPart::Expr(e)   => eval_expr(e, env).to_display(),
            }).collect())
        }
        Expr::Array(items) => Value::Array(items.iter().map(|e| eval_expr(e, env)).collect()),
        Expr::Call(name, args) => {
            let vals: Vec<Value> = args.iter().map(|a| eval_expr(a, env)).collect();
            call_builtin(name, vals, env)
        }
        Expr::BinOp(lhs, op, rhs) => {
            let l = eval_expr(lhs, env); let r = eval_expr(rhs, env);
            match op {
                BinOp::Add => match (&l, &r) {
                    (Value::Str(a), _) => Value::Str(format!("{}{}", a, r.to_display())),
                    (_, Value::Str(b)) => Value::Str(format!("{}{}", l.to_display(), b)),
                    _ => Value::Number(l.as_number() + r.as_number()),
                },
                BinOp::Sub => Value::Number(l.as_number() - r.as_number()),
                BinOp::Mul => Value::Number(l.as_number() * r.as_number()),
                BinOp::Div => {
                    let r = r.as_number();
                    if r == 0.0 { Value::Null } else { Value::Number(l.as_number() / r) }
                    
                }
                BinOp::Mod => {
    let r = r.as_number();
    if r == 0.0 { Value::Null } else { Value::Number(l.as_number() % r) }
}
            }
        }
       Expr::Cmp(lhs, op, rhs) => {
    let l = eval_expr(lhs, env); let r = eval_expr(rhs, env);
    Value::Bool(match op {
        CmpOp::Eq => values_equal(&l, &r),
        CmpOp::Ne => !values_equal(&l, &r),
                CmpOp::Lt => l.as_number() <  r.as_number(),
                CmpOp::Le => l.as_number() <= r.as_number(),
                CmpOp::Gt => l.as_number() >  r.as_number(),
                CmpOp::Ge => l.as_number() >= r.as_number(),
            })
        }
        Expr::Not(e) => Value::Bool(!eval_expr(e, env).as_bool()),
        Expr::And(l, r) => {
            if !eval_expr(l, env).as_bool() { Value::Bool(false) }
            else { Value::Bool(eval_expr(r, env).as_bool()) }
        }
        Expr::Or(l, r) => {
            if eval_expr(l, env).as_bool() { Value::Bool(true) }
            else { Value::Bool(eval_expr(r, env).as_bool()) }
        }
        Expr::NullCoalesce(lhs, rhs) => {
            let l = eval_expr(lhs, env);
            if matches!(l, Value::Null) { eval_expr(rhs, env) } else { l }
        }
        Expr::Index(base, idx) => {
            let b = eval_expr(base, env); let i = eval_expr(idx, env);
            match (b, i) {
                (Value::Array(arr), Value::Number(n)) => arr.get(n as usize).cloned().unwrap_or(Value::Null),
                (Value::Map(mut m), key) => m.shift_remove(&key.to_display()).unwrap_or(Value::Null),
                _ => Value::Null,
            }
        }
        Expr::Map(pairs) => {
            let mut map = IndexMap::new();
            for (k, v) in pairs { map.insert(eval_expr(k, env).to_display(), eval_expr(v, env)); }
            Value::Map(map)
        }
    }
}

fn eval_str_with_vars(s: &str, env: &Env) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        if let Some(end) = rest.find('}') {
            let path: Vec<String> = rest[..end].split('.').map(|s| s.to_string()).collect();
            out.push_str(&env.get_path_str(&path).to_display());
            rest = &rest[end + 1..];
        }
    }
    out.push_str(rest);
    out
}

// ── Codegen 構造体 ────────────────────────────────────────

pub struct Codegen {
    pub env: Env,
    pub talks: HashMap<String, Vec<Talk>>,
    pub selector: TalkSelector,
    pub errors: Vec<(String, String)>,
    pub ghost_dir: PathBuf,
    /// ロード済みSAORI DLLのキャッシュ。IndexMapの挿入順を
    /// 「最近使った順」に保つLRUとして使う（先頭=最古、末尾=最新）。
    /// SAORI_CACHE_LIMITを超えたら先頭からshift_remove_indexし、
    /// SaoriDllのDropでFreeLibrary/unloadが呼ばれてDLLも解放される。
    /// 上限が無いと、台本がdll名を動的に組み立てて毎回異なる文字列を
    /// 渡すようなケースで、呼び出しのたびにDLLがロードされっぱなしになり
    /// 長時間稼働で無視できないメモリ・ハンドルリークになる。
    ///
    /// Arcで持っているのは、request_with_timeoutがタイムアウトした際に
    /// バックグラウンドで走り続けるrequest呼び出しがSaoriDllを使い終える
    /// までDrop（FreeLibrary/unload）を遅らせるため。
    pub saori_cache: IndexMap<String, Arc<crate::saori::SaoriDll>>,
    /// request_with_timeoutがタイムアウトしたSAORI DLL名の集合。
    /// 一度タイムアウトしたDLLは、バックグラウンドに古い呼び出しが
    /// 取り残されている可能性があるため、このロード期間中は二度と
    /// 呼び出さない（多くのSAORI実装は複数スレッドからの同時呼び出しを
    /// 想定していないため）。ゴーストの再読み込み（loadu/load）で
    /// Codegenごと作り直されるとクリアされる。
    saori_timed_out: HashSet<String>,
        /// サンドボックスの根（ゴーストのホーム）。canonical済み。
    /// starts_withでの比較は両辺がcanonicalでないと意味がないため、
    /// ここだけは正規化した形で持つ。
    home_root: PathBuf,
        /// この発話中に既に出力したスコープタグ
    spoken_scopes: HashSet<String>,
    /// 現在のスコープ。Noneは未確定（発話の先頭、または生スクリプト行の直後）
    current_scope: Option<String>,
    /// スコープ切り替え時の\n自動挿入。config.tomlから設定
    pub auto_newline: bool,

}

const LOOP_LIMIT: usize = 2000;
/// format()の幅・精度指定の上限。辞書スクリプトから
/// `${format('%9999999999s', 'x')}`のような桁数を渡されると、
/// 上限なしでは" ".repeat(pad_len)等が数十億文字の確保を試み、
/// アロケータのhandle_alloc_errorで即abortする（catch_unwindでも
/// panic=unwindでも捕まえられない）。LOOP_LIMITやFILE_READ_LIMIT
/// 同様、辞書由来の値は必ずクランプする。
const FORMAT_MAX_WIDTH: usize = 1000;
/// saori_cacheに同時に保持するSAORI DLLの最大数。
/// 通常の台本は数個の固定dll名しか使わないため十分な余裕を持たせつつ、
/// 動的に生成されたdll名が際限なく増えても頭打ちにする。
const SAORI_CACHE_LIMIT: usize = 32;

/// 1回のSAORI request呼び出しの待ち上限。
/// STATEのMutexを保持したまま同期でDLLを呼ぶため、ハングしたSAORIが
/// あってもSSP全体のフリーズを一定時間で打ち切るための上限。
/// （タイムアウト後もバックグラウンド呼び出し自体は走り続けうる。
///   詳細はsaori::request_with_timeoutのコメントを参照）
const SAORI_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// 1回のイベント処理（gen_event）中にget_property/saoriを合計で
/// 呼び出せる回数の上限。
///
/// get_propertyは1回あたり最大800ms程度（sstp::CONNECT_TIMEOUT+IO_TIMEOUT）、
/// saoriは1回あたり最大SAORI_CALL_TIMEOUTブロックしうる。どちらもSTATEの
/// Mutexを保持したまま呼ばれるため、これらをwhile/for等のループ内で
/// 繰り返し呼ぶ台本があると、上限が無ければ「1回あたりの上限×ループ回数
/// （最大LOOP_LIMIT=2000）」までSSP全体をブロックしてしまう。
/// この上限を設けることで、1イベントあたりの最悪ブロック時間を
/// 「1回の上限×MAX_EXTERNAL_CALLS_PER_EVENT」に頭打ちにする。
const MAX_EXTERNAL_CALLS_PER_EVENT: usize = 10;

/// get_property/saoriの呼び出し予算を1消費し、まだ予算内であればtrueを返す。
/// 予算切れの場合は呼び出し元が副作用（実際のSSTP/SAORI呼び出し）を
/// 起こさずに空の値を返すこと。
///
/// カウンタはEnv側（Codegenインスタンスごと）に持たせている。
/// call_builtinは`&mut self`を取らない自由関数（condの評価などで使う
/// &Env版のeval_expr経由でも呼ばれるため）なので、`&Env`だけで
/// 読み書きできるようCellで持つ。イベント開始時にreset_runtime_stateで
/// 0に戻る。
fn consume_external_call_budget(env: &Env) -> bool {
    let prev = env.external_call_count.get();
    env.external_call_count.set(prev + 1);
    prev < MAX_EXTERNAL_CALLS_PER_EVENT
}

enum FlowControl {
    Return(Value),
    Break,
    Continue,
}

impl Codegen {
    pub fn new(characters: HashMap<String, String>, talks: HashMap<String, Vec<Talk>>, ghost_dir: PathBuf) -> Self {
        let canonical = ghost_dir.canonicalize().unwrap_or_else(|_| ghost_dir.clone());
        let home_root = compute_home_dir(&canonical);
        Self {
            env: Env::new(characters),
            talks,
            selector: TalkSelector::new(),
            errors: vec![],
            ghost_dir,
            saori_cache: IndexMap::new(),
            saori_timed_out: HashSet::new(),
            home_root,
                spoken_scopes:HashSet::new(),
    current_scope: None,
    auto_newline: true,
            
        }
    }

    // ── ローカルスコープに reference/now を注入 ──────────

    fn inject_talk_locals(
        &mut self,
        refs: &HashMap<String, String>,
        virtual_time: Option<(i32, u32, u32, u32, u32, u32)>,
    ) {
        let ref_map: IndexMap<String, Value> = refs.iter()
            .map(|(k, v)| (k.clone(), Value::Str(v.clone())))
            .collect();
        self.env.set_local("reference", Value::Map(ref_map));

        let (y, mo, d, h, mi, s, wd) = match virtual_time {
            Some((y, mo, d, h, mi, s)) => {
                let wd = NaiveDate::from_ymd_opt(y, mo, d)
                    .map(|dt| dt.weekday().num_days_from_monday())
                    .unwrap_or(0);
                (y as f64, mo as f64, d as f64, h as f64, mi as f64, s as f64, wd as f64)
            }
            None => {
                use chrono::FixedOffset;
                let jst = FixedOffset::east_opt(9 * 3600).unwrap();
                let now = Utc::now().with_timezone(&jst);
                (
                    now.year() as f64, now.month() as f64, now.day() as f64,
                    now.hour() as f64, now.minute() as f64, now.second() as f64,
                    now.weekday().num_days_from_monday() as f64,
                )
            }
        };

        let now_map: IndexMap<String, Value> = [
            ("年".to_string(), Value::Number(y)),
            ("月".to_string(), Value::Number(mo)),
            ("日".to_string(), Value::Number(d)),
            ("時".to_string(), Value::Number(h)),
            ("分".to_string(), Value::Number(mi)),
            ("秒".to_string(), Value::Number(s)),
            ("曜日".to_string(), Value::Number(wd)),
        ].into_iter().collect();
        self.env.set_local("now", Value::Map(now_map));
    }


/// イベント開始時に、前回の実行が残した実行時状態を捨てる。
    ///
    /// requestはcatch_unwindでパニックを握り潰し、lock_stateはMutexの
    /// poisonを無視するため、gen_event中に落ちても同じCodegenが次の
    /// イベントで再利用される。env側（ローカルスコープ・call_depth）は
    /// reset_runtime_stateで戻るが、errorsはCodegen側のフィールドなので
    /// そこに含まれない。放置すると、パニック時点までに積まれたエラーが
    /// 次のイベントの応答に無関係なまま載る。
    ///
    /// 正常系ではhandle_request側がdrainするので、ここに来る時点で
    /// errorsは既に空であり、挙動は変わらない。
    fn reset_for_event(&mut self) {

        self.env.reset_runtime_state();
        self.errors.clear();
         self.spoken_scopes.clear();
self.current_scope = None;
    }

    // ── gen_event: 条件フィルタ→選択→実行（lib.rsから呼ぶ）
pub fn gen_event(
     &mut self,
    event: &str,
    candidates: &[Talk],
    refs: &HashMap<String, String>,
    virtual_time: Option<(i32, u32, u32, u32, u32, u32)>,
)  -> Option<String> {
 

    // 条件フィルタ
     self.reset_for_event();
              self.spoken_scopes.clear();
self.current_scope = None;
    self.env.push_scope();
    self.inject_talk_locals(refs, virtual_time);

    let alive = self.filter_alive(candidates);

    if alive.is_empty() {
        self.env.pop_scope();
        return None;
    }
    let talk: Talk = match self.selector.select_alive(event, &alive) {
        Some(t) => t.clone(),
        None => { self.env.pop_scope(); return None; }
    };
    append_log!(format!("enter talk: {} (event={})", talk.event, event));
    let mut out = String::new();

    for stmt in &talk.body {
        match self.gen_stmt(stmt, &mut out) {
            Some(FlowControl::Return(val)) => { out.push_str(&val.to_display()); break; }
            Some(FlowControl::Break) | Some(FlowControl::Continue) => break,
            None => {}
        }
    }
    append_log!(format!("gen_event {} step2: out=[{}]", event, out));

    self.env.pop_scope();

    if out.is_empty() {
        append_log!(format!("★EMPTY OUTPUT★ event={}, talk.event={}, body_len={}", event, talk.event, talk.body.len()));
        // 空出力を作者向けエラーとして記録（if分岐やcond条件のミスに気付けるように）
        self.errors.push((
            "notice".to_string(),
            format!("イベント「{}」のトーク「{}」が選ばれましたが、出力が空でした（if分岐やcond条件を確認してください）", event, talk.event)
        ));
        return None;
    }
    out.push_str("\\e");
    Some(out)
}

    // ── gen_talk: テストから直接1件を実行する用 ──────────

    #[cfg(test)]
    pub fn gen_talk(
        &mut self,
        talk: &Talk,
        refs: &HashMap<String, String>,
        virtual_time: Option<(i32, u32, u32, u32, u32, u32)>,
    ) -> String {
        #[cfg(debug_assertions)]
        append_log!(format!("globals keys: {:?}", self.env.globals.keys().collect::<Vec<_>>()));
        #[cfg(debug_assertions)]
        append_log!(format!("save value: {:?}", self.env.globals.get("save")));
                 self.spoken_scopes.clear();
self.current_scope = None;
        self.env.push_scope();
        self.inject_talk_locals(refs, virtual_time);

        let mut out = String::new();
        for stmt in &talk.body {
            match self.gen_stmt(stmt, &mut out) {
                Some(FlowControl::Return(val)) => { out.push_str(&val.to_display()); break; }
                Some(FlowControl::Break) | Some(FlowControl::Continue) => break,
                None => {}
            }
        }

        self.env.pop_scope();
        #[cfg(debug_assertions)]
        append_log!(format!("errors: {:?}", self.errors));
        out.push_str("\\e");
        out
    }

    // ── gen_stmt ──────────────────────────────────────────

    fn gen_stmt(&mut self, stmt: &Stmt, out: &mut String) -> Option<FlowControl> {
        #[cfg(debug_assertions)]
        append_log!(format!("gen_stmt: {:?}", stmt));
        match stmt {
            Stmt::Dialogue(line) => { self.gen_dialogue(line, out); None }

            Stmt::Let(name, expr) => {
                if name == "__skip__" {
                    if let Expr::Str(bad) = expr {
                        self.errors.push(("warning".to_string(), format!("認識できない文です: 「{}」", bad)));
                    }
                    return None;
                }
                let val = self.eval_expr_full(expr);
                append_log!(format!("let {} = {:?}", name, val));
                self.env.set_local(name, val);
                None
            }
// codegen.rs — gen_stmt の Stmt::Global

Stmt::Global(path, op, expr) => {
    let val = self.eval_expr_full(expr);
    let resolved = self.resolve_path_segments(path);              // ★追加
    append_log!(format!("before history: save={:?}", self.env.globals.get("save")));
    append_log!(format!("set_path: {:?} {:?}", resolved, val));    // ★path→resolved
    self.env.set_path_resolved(&resolved, op, val);                // ★set_path→set_path_resolved
    None
}
// codegen.rs — gen_stmt の Stmt::Assign

Stmt::Assign(path, op, expr) => {
    let val = self.eval_expr_full(expr);
    match path.as_slice() {
        [PathSegment::Key(key)] => self.env.set_var(key, op, val),
        _ => {
            let resolved = self.resolve_path_segments(path);           // ★追加
            self.env.set_var_path_resolved(&resolved, op, val);        // ★変更
        }
    }
    None
}

            Stmt::If(cond, then_body, else_body) => {
                self.env.push_scope();
                let result = if self.eval_expr_full(cond).as_bool() {
                    self.run_stmts(then_body, out)
                } else if let Some(eb) = else_body {
                    self.run_stmts(eb, out)
                } else { None };
                self.env.pop_scope();
                result
            }

Stmt::For { init, cond, step, body } => {
    self.env.push_scope();
    self.gen_stmt(init, out);
    let mut count = 0;
    let mut result = None;
    loop {
        if !self.eval_expr_full(cond).as_bool() { break; }
        if count >= LOOP_LIMIT {
            self.errors.push((
                "warning".to_string(),
                format!("for文がループ上限（{}回）に達したため打ち切りました。無限ループになっていないか確認してください", LOOP_LIMIT),
            ));
            break;
        }
        count += 1;
        self.env.push_scope();
        match self.run_stmts(body, out) {
            Some(FlowControl::Return(v)) => { self.env.pop_scope(); result = Some(FlowControl::Return(v)); break; }
            Some(FlowControl::Break)     => { self.env.pop_scope(); break; }
            Some(FlowControl::Continue)  => { self.env.pop_scope(); self.step_for(step); continue; }
            None => {}
        }
        self.env.pop_scope();
        self.step_for(step);
    }
    self.env.pop_scope();
    result
}

   Stmt::ForEach { collection, key, value, body } => {
    let col = self.eval_expr_full(collection);
    let mut result = None;
    match col {
        Value::Array(arr) => {
            if arr.len() > LOOP_LIMIT {
                self.errors.push((
                    "warning".to_string(),
                    format!("foreach文の配列要素数（{}）がループ上限（{}）を超えたため、以降の要素を打ち切りました", arr.len(), LOOP_LIMIT),
                ));
            }
            for (i, v) in arr.iter().enumerate().take(LOOP_LIMIT) {
                self.env.push_scope();
                self.env.set_local(key, Value::Number(i as f64));
                if let Some(vn) = value { self.env.set_local(vn, v.clone()); }
                match self.run_stmts(body, out) {
                    Some(FlowControl::Return(v)) => { self.env.pop_scope(); result = Some(FlowControl::Return(v)); break; }
                    Some(FlowControl::Break)     => { self.env.pop_scope(); break; }
                    Some(FlowControl::Continue)  => { self.env.pop_scope(); continue; }
                    None => {}
                }
                self.env.pop_scope();
            }
        }
        Value::Map(map) => {
            if map.len() > LOOP_LIMIT {
                self.errors.push((
                    "warning".to_string(),
                    format!("foreach文のMap要素数（{}）がループ上限（{}）を超えたため、以降の要素を打ち切りました", map.len(), LOOP_LIMIT),
                ));
            }
            for (i, (k, v)) in map.iter().enumerate() {
                if i >= LOOP_LIMIT { break; }
                self.env.push_scope();
                self.env.set_local(key, Value::Str(k.clone()));
                if let Some(vn) = value { self.env.set_local(vn, v.clone()); }
                match self.run_stmts(body, out) {
                    Some(FlowControl::Return(v)) => { self.env.pop_scope(); result = Some(FlowControl::Return(v)); break; }
                    Some(FlowControl::Break)     => { self.env.pop_scope(); break; }
                    Some(FlowControl::Continue)  => { self.env.pop_scope(); continue; }
                    None => {}
                }
                self.env.pop_scope();
            }
        }
        _ => {}
    }
    result
}
Stmt::While(cond, body) => {
    let mut count = 0;
    let mut result = None;
    loop {
        if !self.eval_expr_full(cond).as_bool() { break; }
        if count >= LOOP_LIMIT {
            self.errors.push((
                "warning".to_string(),
                format!("while文がループ上限（{}回）に達したため打ち切りました。無限ループになっていないか確認してください", LOOP_LIMIT),
            ));
            break;
        }
        count += 1;
        self.env.push_scope();
        match self.run_stmts(body, out) {
            Some(FlowControl::Return(v)) => { self.env.pop_scope(); result = Some(FlowControl::Return(v)); break; }
            Some(FlowControl::Break)     => { self.env.pop_scope(); break; }
            Some(FlowControl::Continue)  => { self.env.pop_scope(); continue; }
            None => {}
        }
        self.env.pop_scope();
    }
    result
}

            Stmt::Call(expr) => {
                append_log!(format!("funcs keys:{:?}", self.env.funcs.keys().collect::<Vec<_>>()));
                let name = match expr {
                    Expr::Var(path) => {
                        let val = self.eval_expr_full(&Expr::Var(path.clone()));
                        match val {
                            Value::Str(s) if !s.is_empty() => s,
                            _ => path.join("."),
                        }
                    }
                    _ => self.eval_expr_full(expr).to_display(),
                };

                

                if self.env.funcs.contains_key(&name) {
                    self.call_func_stmt(&name, vec![], out);
} else if let Some(candidates) = self.talks.get(&name).cloned() {
    if !candidates.is_empty() {
        if self.env.call_depth > 100 {
            append_log!(format!("talk call depth limit: {}", name));
            return None;
        }
        self.env.call_depth += 1;
let alive = self.filter_alive(&candidates);

        if alive.is_empty() {
            self.env.call_depth -= 1;
            // gen_event（SSPからのイベント）の全滅は時間帯cond等で
            // 日常的に起こる正常系だが、call は作者が「ここで喋る」と
            // 明示した場所なので、無言になったら知らせる。
            self.errors.push((
                "notice".to_string(),
                format!("call「{}」の候補が全てcondで除外され、何も出力されませんでした", name)
            ));
        } else {
            match self.selector.select_alive(&name, &alive) {
                Some(t) => {
                    let talk = t.clone();
                    append_log!(format!("enter talk (call): {}", talk.event));
                    self.env.push_scope();

                    let result = self.run_stmts(&talk.body, out);
                    self.env.pop_scope();
                    self.env.call_depth -= 1;

                    if let Some(FlowControl::Return(val)) = result {
                        append_log!(format!("call return val: {:?}", val.to_display()));
                        out.push_str(&val.to_display());
                    }
                }
                None => {
                    self.env.call_depth -= 1;
                    self.errors.push((
                        "notice".to_string(),
                        format!("call「{}」の候補選択に失敗し、何も出力されませんでした", name)
                    ));
                }
            }
        }
    }
}
                None
            }

            Stmt::FuncDef { name, params, body } => {
                self.env.funcs.insert(name.clone(), (params.clone(), body.clone()));
                None
            }

            Stmt::Return(expr) => Some(FlowControl::Return(self.eval_expr_full(expr))),
            Stmt::Break    => Some(FlowControl::Break),
            Stmt::Continue => Some(FlowControl::Continue),

            Stmt::Match { expr, arms } => {
                let val = self.eval_expr_full(expr);
                let _val_str = val.to_display();
                for arm in arms {
           // 変更後
let matched = arm.patterns.iter().any(|p| match p {
    MatchPattern::Wildcard => true,
    MatchPattern::Value(e) => {
        let pval = self.eval_expr_full(e);
        match (&val, &pval) {
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::Str(a), Value::Number(b)) =>
                a.parse::<f64>().map(|a| a == *b).unwrap_or(false)
                || a == &b.to_string().trim_end_matches(".0").to_string(),
            _ => values_equal(&val, &pval),
        }
    }
});
                    if matched {
                        self.env.push_scope();
                        let result = self.run_stmts(&arm.body, out);
                        self.env.pop_scope();
                        return result;
                    }
                }
                None
            }
        }
    }

    // ── eval_expr_full ────────────────────────────────────

    pub(crate) fn eval_expr_full(&mut self, expr: &Expr) -> Value {
        match expr {
// codegen.rs — eval_expr_full 内
Expr::Call(fname, args) => {
    // choose は遅延評価が必須。vals の一括評価に入る前に条件を見て、
    // 選ばれた側の式だけを一度だけ評価する。
    // こうしないと cond やsaori()/log()等の副作用が余計に発火したり、
    // 選ばれなかった側の式まで(discardされるとはいえ)一度評価されてしまう。
    if fname == "choose" {
        let cond = args.get(0).map(|a| self.eval_expr_full(a).as_bool()).unwrap_or(false);
        return if cond {
            args.get(1).map(|a| self.eval_expr_full(a)).unwrap_or(Value::Null)
        } else {
            args.get(2).map(|a| self.eval_expr_full(a)).unwrap_or(Value::Null)
        };
    }

    let vals: Vec<Value> = args.iter().map(|a| self.eval_expr_full(a)).collect();
    if self.env.funcs.contains_key(fname.as_str()) {
        self.call_func(fname, vals)
  

} else if self.talks.contains_key(fname.as_str()) {
    let mut tmp_out = String::new();
    if let Some(candidates) = self.talks.get(fname.as_str()).cloned() {
        if self.env.call_depth > 100 {
            append_log!(format!("talk call depth limit (expr): {}", fname));
            return Value::Str(String::new());
        }
        self.env.call_depth += 1;
        let alive = self.filter_alive(&candidates);

        if !alive.is_empty() {
            match self.selector.select_alive(fname, &alive) {
                Some(t) => {
                    let talk = t.clone();
                    self.env.push_scope();
                    append_log!(format!("enter talk (call): {}", talk.event));
                    let saved_scope = self.current_scope.take();
                    let saved_spoken = std::mem::take(&mut self.spoken_scopes);
                    match self.run_stmts(&talk.body, &mut tmp_out) {
                        Some(FlowControl::Return(val)) => { tmp_out.push_str(&val.to_display()); }
                        _ => {}
                    }
                    self.current_scope = saved_scope;
                    self.spoken_scopes = saved_spoken;
                    self.env.pop_scope();
                }
                None => {
                    self.errors.push((
                        "notice".to_string(),
                        format!("「{}()」の候補選択に失敗し、空文字列になりました", fname)
                    ));
                }
            }
               } else {
            self.errors.push((
                "notice".to_string(),
                format!("「{}()」の候補が全てcondで除外され、空文字列になりました", fname)
            ));
        }
        self.env.call_depth -= 1;
        

    }
    Value::Str(tmp_out)
}
else {
        match fname.as_str() {
                   
      "saori" => {
    if vals.is_empty() { return Value::Str(String::new()); }
    let dll_name = vals[0].to_display();
    let args: Vec<String> = vals[1..].iter().map(|v| v.to_display()).collect();

    if self.saori_timed_out.contains(&dll_name) {
        // 一度タイムアウトしたSAORIは、このロード期間中は再呼び出ししない。
        // バックグラウンドに取り残された古い呼び出しと新しい呼び出しが
        // 同時に同一DLLのrequest_fnへ入ることを避けるため
        // （詳細はsaori::request_with_timeout / SaoriDllのSync実装コメント参照）。
        self.errors.push((
            "warning".to_string(),
            format!("SAORI「{}」は以前応答がタイムアウトしたため、このロード期間中は呼び出しを停止しています（ゴーストの再読み込みで復帰します）", dll_name)
        ));
        return Value::Array(vec![]);
    }

    if !consume_external_call_budget(&self.env) {
        // ループ内でsaori/get_propertyを繰り返し呼ぶ台本が、STATEのMutexを
        // 保持したまま実質無制限にSSP全体をブロックしないための上限
        // （詳細はMAX_EXTERNAL_CALLS_PER_EVENT参照）。
        self.errors.push((
            "warning".to_string(),
            format!("1イベントあたりの外部呼び出し上限（{}回）に達したため「{}」の呼び出しを無視しました", MAX_EXTERNAL_CALLS_PER_EVENT, dll_name)
        ));
        return Value::Array(vec![]);
    }

    if let Some(idx) = self.saori_cache.get_index_of(&dll_name) {
        // LRU: 使ったエントリを末尾（最新）に移動する
        let last = self.saori_cache.len() - 1;
        self.saori_cache.move_index(idx, last);
    } else {
        // joinは右辺が絶対パスだと左辺を捨てるため、
        // 検査しないと saori('C:\\evil.dll') がそのまま読み込まれる。
        // 基準はhome_rootではなくghost_dir（既存台本が
        // 'saori.dll' のようにmaster相対で書いているため）。
        let segs = match validate_rel_path(&dll_name) {
            Ok(s) => s,
            Err(msg) => {
                self.errors.push(("error".to_string(), format!("{}: 「{}」", msg, dll_name)));
                return Value::Str(String::new());
            }
        };
        let mut dll_path = self.ghost_dir.clone();
        for s in &segs { dll_path.push(s); }
        match crate::saori::SaoriDll::load(&dll_path) {
            Ok(dll) => {
                if self.saori_cache.len() >= SAORI_CACHE_LIMIT {
                    // 先頭（最も長く使われていない）を追い出す。
                    // shift_removeでArcの参照が1つ減る。バックグラウンドで
                    // まだ使用中でなければここでDrop（unload_fn + FreeLibrary）が走る。
                    if let Some((evicted_name, _)) = self.saori_cache.shift_remove_index(0) {
                        append_log!(format!("saori_cache上限到達、追い出し: {}", evicted_name));
                    }
                }
                self.saori_cache.insert(dll_name.clone(), Arc::new(dll));
            }
            Err(e)  => {
                append_log!(format!("saori load failed: {}: {}", dll_name, e));
                self.errors.push(("error".to_string(), e));
                return Value::Str(String::new());
            }
        }
    }
    let dll = self.saori_cache[&dll_name].clone();
    let result = match crate::saori::request_with_timeout(dll, args, SAORI_CALL_TIMEOUT) {
        Some(r) => r,
        None => {
            append_log!(format!("saori request timeout: {}", dll_name));
            self.errors.push((
                "warning".to_string(),
                format!("SAORI「{}」の応答がタイムアウト（{}秒）したため打ち切りました", dll_name, SAORI_CALL_TIMEOUT.as_secs())
            ));
            // 以後このロード期間中は呼び出さない。キャッシュからも外し、
            // 取り残されたバックグラウンド呼び出しがArcを持つ間だけ
            // DLLの実体を生かしておく。
            self.saori_timed_out.insert(dll_name.clone());
            self.saori_cache.shift_remove(&dll_name);
            HashMap::new()
        }
    };
    let mut keys: Vec<usize> = result.keys().filter_map(|k| k.parse().ok()).collect();
    keys.sort();
    Value::Array(keys.iter().map(|k| Value::Str(result.get(&k.to_string()).cloned().unwrap_or_default())).collect())
}

                        "days_since" => {
                            if vals.len() >= 3 {
                                let y = vals[0].as_number() as i32;
                                let m = vals[1].as_number() as u32;
                                let d = vals[2].as_number() as u32;
                                if let Some(start) = NaiveDate::from_ymd_opt(y, m, d) {
                                    let today = match self.env.get("now") {
                                        Value::Map(ref m) => {
                                            let ty = m.get("年").map(|v| v.as_number() as i32).unwrap_or(0);
                                            let tm = m.get("月").map(|v| v.as_number() as u32).unwrap_or(1);
                                            let td = m.get("日").map(|v| v.as_number() as u32).unwrap_or(1);
                                            NaiveDate::from_ymd_opt(ty, tm, td).unwrap_or_else(|| {
                                                use chrono::FixedOffset;
                                                Utc::now().with_timezone(&FixedOffset::east_opt(9*3600).unwrap()).date_naive()
                                            })
                                        }
                                        _ => {
                                            use chrono::FixedOffset;
                                            Utc::now().with_timezone(&FixedOffset::east_opt(9*3600).unwrap()).date_naive()
                                        }
                                    };
                                    Value::Number((today - start).num_days() as f64)
                                } else { Value::Null }
                            } else { Value::Null }
                        }
"days_between" => {
    if vals.len() >= 6 {
        let d1 = NaiveDate::from_ymd_opt(
            vals[0].as_number() as i32, vals[1].as_number() as u32, vals[2].as_number() as u32
        );
        let d2 = NaiveDate::from_ymd_opt(
            vals[3].as_number() as i32, vals[4].as_number() as u32, vals[5].as_number() as u32
        );
        match (d1, d2) {
            (Some(a), Some(b)) => Value::Number((b - a).num_days() as f64),
            _ => Value::Null,
        }
    } else { Value::Null }
}
                        
"is_null" => {
    Value::Bool(matches!(vals.get(0).unwrap_or(&Value::Null), Value::Null))
}
// eval_expr_full の Expr::Call の中、saori/choose/days_since と並べて追加
"log" => {
    let msg = vals.iter().map(|v| v.to_display()).collect::<Vec<_>>().join(", ");
    append_script_log(&format!("[SCRIPT] {}", msg), &self.ghost_dir);
    Value::Null
}
"talk_exists" => {
    let name = vals.get(0).map(|v| v.to_display()).unwrap_or_default();
    Value::Bool(!name.is_empty() && (self.talks.contains_key(&name) || self.env.funcs.contains_key(&name)))
}


    "file_read" => {
    let p = vals.get(0).map(|v| v.to_display()).unwrap_or_default();
    let enc = vals.get(1).map(|v| v.to_display().to_lowercase()).unwrap_or_default();
    let full = match self.resolve_file(&p, true) {
        Ok((f, _)) => f,
        Err(e) => { self.file_reject(&e); return Value::Null; }
    };
    match std::fs::metadata(&full) {
        Ok(m) if m.is_dir() => {
            self.file_warn(format!("フォルダは読み込めません: 「{}」", p));
            return Value::Null;
        }
        Ok(m) if m.len() > FILE_READ_LIMIT => {
            // 切り詰めて返すと、それが完全なデータだと誤解されるのでNullにする
            self.file_warn(format!(
                "ファイルが大きすぎます（{}バイト、上限{}バイト）: 「{}」", m.len(), FILE_READ_LIMIT, p
            ));
            return Value::Null;
        }
        Err(_e) => {
            self.file_warn(format!("ファイル情報が取得できません: 「{}」", p));
            return Value::Null;
        }
        _ => {}
    }
    match std::fs::read(&full) {
        Ok(bytes) => match decode_text(&bytes, &enc) {
            Ok(s) => Value::Str(s),
            Err(msg) => { self.file_warn(format!("{}: 「{}」", msg, p)); Value::Null }
        },
        Err(_e) => {
            self.file_warn(format!("ファイルが読み込めません: 「{}」", p));
            Value::Null
        }
    }
}

"file_write" => {
    let p = vals.get(0).map(|v| v.to_display()).unwrap_or_default();
    let content = vals.get(1).map(|v| v.to_display()).unwrap_or_default();
    let enc = vals.get(2).map(|v| v.to_display().to_lowercase()).unwrap_or_default();
    let full = match self.resolve_writable(&p) {
        Some(f) => f,
        None => return Value::Bool(false),
    };
    let bytes = match encode_text(&content, &enc) {
        Ok(b) => b,
        Err(msg) => { self.file_warn(format!("{}: 「{}」", msg, p)); return Value::Bool(false); }
    };
    // save_globalsと同じくtmp+renameにして、書き込み中に落ちても
    // 既存ファイルが半端な内容で残らないようにする
    let mut tmp_name = full.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".minato.tmp");
    let tmp = full.with_file_name(tmp_name);
    if let Err(e) = self::ensure_not_link(&tmp, &p) {
        self.file_reject(&e);
        return Value::Bool(false);
    }
    match std::fs::write(&tmp, &bytes) {
        Ok(_) => match std::fs::rename(&tmp, &full) {
            Ok(_) => Value::Bool(true),
            Err(_e) => {
                let _ = std::fs::remove_file(&tmp);
                self.file_warn(format!("ファイルの保存に失敗しました: 「{}」", p));
                Value::Bool(false)
            }
        },
        Err(_e) => {
            self.file_warn(format!("ファイルの書き込みに失敗しました: 「{}」", p));
            Value::Bool(false)
        }
    }
}

"file_append" => {
    let p = vals.get(0).map(|v| v.to_display()).unwrap_or_default();
    let content = vals.get(1).map(|v| v.to_display()).unwrap_or_default();
    let enc = vals.get(2).map(|v| v.to_display().to_lowercase()).unwrap_or_default();
    let full = match self.resolve_writable(&p) {
        Some(f) => f,
        None => return Value::Bool(false),
    };
    let bytes = match encode_text(&content, &enc) {
        Ok(b) => b,
        Err(msg) => { self.file_warn(format!("{}: 「{}」", msg, p)); return Value::Bool(false); }
    };
    // 追記はtmp+renameにできない（全体を読み直すことになる）ので直接開く。
    // 書き込み途中で落ちると半端な行が残りうるが、ログ用途を想定しているので許容する。
    use std::io::Write;
    let opened = std::fs::OpenOptions::new().create(true).append(true).open(&full);
    match opened {
        Ok(mut f) => match f.write_all(&bytes) {
            Ok(_) => Value::Bool(true),
            Err(_e) => {
                self.file_warn(format!("ファイルの追記に失敗しました: 「{}」", p));
                Value::Bool(false)
            }
        },
        Err(_e) => {
            self.file_warn(format!("ファイルが開けません: 「{}」", p));
            Value::Bool(false)
        }
    }
}

"file_move" => {
    let from_p = vals.get(0).map(|v| v.to_display()).unwrap_or_default();
    let to_p   = vals.get(1).map(|v| v.to_display()).unwrap_or_default();
    let overwrite = vals.get(2).map(|v| v.as_bool()).unwrap_or(false);

    // 移動元も「書き込み」（消える）なので、読み込み範囲ではなく
    // 書き込み範囲で判定する
    let (from_full, from_segs) = match self.resolve_file(&from_p, true) {
        Ok(v) => v,
        Err(e) => { self.file_reject(&e); return Value::Bool(false); }
    };
    if let Err(msg) = check_writable(&from_segs) {
        self.file_reject(&PathReject::Denied(msg));
        return Value::Bool(false);
    }
    if from_full.is_dir() {
        self.file_warn(format!("フォルダは移動できません: 「{}」", from_p));
        return Value::Bool(false);
    }
    let to_full = match self.resolve_writable(&to_p) {
        Some(f) => f,
        None => return Value::Bool(false),
    };
    // fs::renameはWindowsで既存ファイルを黙って上書きするため、
    // 非上書きのデフォルトは明示的なexists()チェックでしか実現できない
       // 大小文字違い等で同じ実体を指す場合、exists()は真になるが
    // 上書き事故にはならない。ここでガードすると
    // 「a.txt → A.TXT」のような綴り変更ができなくなる。
    let same_file = to_full.exists()
        && to_full.canonicalize().map(|t| t == from_full).unwrap_or(false);
    if to_full.exists() && !overwrite && !same_file {
        self.file_warn(format!(
            "移動先に既にファイルがあります（上書きするなら第3引数にtrueを指定してください）: 「{}」", to_p
        ));
        return Value::Bool(false);
    }
    match std::fs::rename(&from_full, &to_full) {
        Ok(_) => Value::Bool(true),
        Err(_e) => {
            self.file_warn(format!("ファイルの移動に失敗しました: 「{}」→「{}」", from_p, to_p));
            Value::Bool(false)
        }
    }
}


"format" => {
    let fmt = vals.get(0).map(|v| v.to_display()).unwrap_or_default();
    let mut result = String::new();
    let mut args_iter = vals.iter().skip(1);
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            result.push(c);
            continue;
        }
        // % の次を見る
        let mut zero_pad = false;
        let mut width = 0usize;
        let mut precision: Option<usize> = None;

        // ゼロ埋め
        if chars.peek() == Some(&'0') {
            zero_pad = true;
            chars.next();
        }
        // 幅
        // 桁数に上限がないと、大量の数字（例: %9999999999999999999s）で
        // usizeの乗算オーバーフローや、後続の"..".repeat(width)による
        // 巨大なメモリ確保（アロケータの即abortに直結）を招くため、
        // saturatingで演算しつつFORMAT_MAX_WIDTHでクランプする。
        while let Some(&d) = chars.peek() {
            if d.is_ascii_digit() {
                width = width.saturating_mul(10)
                    .saturating_add(d as usize - '0' as usize)
                    .min(FORMAT_MAX_WIDTH);
                chars.next();
            } else { break; }
        }
        // 精度
if chars.peek() == Some(&'.') {
    chars.next();
    let mut prec = 0usize;
    while let Some(&d) = chars.peek() {
        if d.is_ascii_digit() {
            prec = prec.saturating_mul(10)
                .saturating_add(d as usize - '0' as usize)
                .min(FORMAT_MAX_WIDTH);
            chars.next();
        } else { break; }
    }
    precision = Some(prec);
}

// 型指定子
match chars.next() {
Some('d') => {
    let n = args_iter.next().map(|v| v.as_number() as i64).unwrap_or(0);
    let formatted = format!("{}", n);
    if width > 0 && formatted.chars().count() < width {
        let pad_len = width - formatted.chars().count();
        if zero_pad {
            if let Some(rest) = formatted.strip_prefix('-') {
                result.push('-');
                result.push_str(&"0".repeat(pad_len));
                result.push_str(rest);
            } else {
                result.push_str(&"0".repeat(pad_len));
                result.push_str(&formatted);
            }
        } else {
            result.push_str(&" ".repeat(pad_len));
            result.push_str(&formatted);
        }
    } else {
        result.push_str(&formatted);
    }
}
Some('f') => {
    let n = args_iter.next().map(|v| v.as_number()).unwrap_or(0.0);
    let prec = precision.unwrap_or(6);
    let formatted = format!("{:.prec$}", n, prec = prec);
    if width > 0 && formatted.chars().count() < width {
        let pad_len = width - formatted.chars().count();
     
        if zero_pad {
            // 符号を保持したまま、符号の直後にゼロを挿入する
            if let Some(rest) = formatted.strip_prefix('-') {
                result.push('-');
                result.push_str(&"0".repeat(pad_len));
                result.push_str(rest);
            } else {
                 
                result.push_str(&"0".repeat(pad_len));
                result.push_str(&formatted);
            }
        } else {
            result.push_str(&" ".repeat(pad_len));
            result.push_str(&formatted);
        }
    } else {
        result.push_str(&formatted);
    }
}
Some('s') => {
    let s = args_iter.next().map(|v| v.to_display()).unwrap_or_default();
    if width > 0 && s.chars().count() < width {
        let pad_len = width - s.chars().count();
        result.push_str(&" ".repeat(pad_len));
        result.push_str(&s);
    } else {
        result.push_str(&s);
    }
}
            Some('%') => result.push('%'),
            Some(other) => { result.push('%'); result.push(other); }
            None => result.push('%'),
        }
    }
    Value::Str(result)
}
                        _ => call_builtin(fname, vals, &self.env),
                    }
                }
            }

            Expr::BinOp(lhs, op, rhs) => {
                let l = self.eval_expr_full(lhs); let r = self.eval_expr_full(rhs);
                match op {
                    BinOp::Add => match (&l, &r) {
                        (Value::Str(a), _) => Value::Str(format!("{}{}", a, r.to_display())),
                        (_, Value::Str(b)) => Value::Str(format!("{}{}", l.to_display(), b)),
                        _ => Value::Number(l.as_number() + r.as_number()),
                    },
                    BinOp::Sub => Value::Number(l.as_number() - r.as_number()),
                    BinOp::Mul => Value::Number(l.as_number() * r.as_number()),
                    BinOp::Div => {
                        let r = r.as_number();
                        if r == 0.0 { self.errors.push(("warning".to_string(), "ゼロ除算が発生しました".to_string())); Value::Null }
                        else { Value::Number(l.as_number() / r) }
                    }
                    BinOp::Mod => {
    let r = r.as_number();
    if r == 0.0 { self.errors.push(("warning".to_string(), "ゼロ除算が発生しました".to_string())); Value::Null }
    else { Value::Number(l.as_number() % r) }
}
                }
            }

            Expr::Index(base, idx) => {
                let b = self.eval_expr_full(base); let i = self.eval_expr_full(idx);
                match (b, i) {
                    (Value::Array(arr), Value::Number(n)) => {
                        let idx = n as usize;
                        if idx >= arr.len() {
                            self.errors.push(("warning".to_string(), format!("配列の範囲外アクセス [{}] (長さ: {})", idx, arr.len())));
                            Value::Null
                        } else { arr[idx].clone() }
                    }
                    (Value::Map(map), key) => {
                        let k = key.to_display();
                        if !map.contains_key(&k) {
                            self.errors.push(("notice".to_string(), format!("Mapに存在しないキー \"{}\"", k)));
                        }
                        map.get(&k).cloned().unwrap_or(Value::Null)
                    }
                    _ => {
                        self.errors.push(("warning".to_string(), "添字アクセスの対象が配列でもMapでもない".to_string()));
                        Value::Null
                    }
                }
            }

            Expr::InterpolatedStr(parts) => {
                let parts = parts.clone();
                Value::Str(self.eval_parts(&parts))
            }
            Expr::NullCoalesce(lhs, rhs) => {
                let l = self.eval_expr_full(lhs);
                if matches!(l, Value::Null) { self.eval_expr_full(rhs) } else { l }
            }
      // 変更後
Expr::Cmp(lhs, op, rhs) => {
    let l = self.eval_expr_full(lhs); let r = self.eval_expr_full(rhs);
    Value::Bool(match op {
        CmpOp::Eq => values_equal(&l, &r),
        CmpOp::Ne => !values_equal(&l, &r),
                    CmpOp::Lt => l.as_number() <  r.as_number(),
                    CmpOp::Le => l.as_number() <= r.as_number(),
                    CmpOp::Gt => l.as_number() >  r.as_number(),
                    CmpOp::Ge => l.as_number() >= r.as_number(),
                })
            }
            Expr::Not(e) => Value::Bool(!self.eval_expr_full(e).as_bool()),
            Expr::And(l, r) => {
                let lv = self.eval_expr_full(l);
                if !lv.as_bool() { Value::Bool(false) } else { Value::Bool(self.eval_expr_full(r).as_bool()) }
            }
            Expr::Or(l, r) => {
                let lv = self.eval_expr_full(l);
                if lv.as_bool() { Value::Bool(true) } else { Value::Bool(self.eval_expr_full(r).as_bool()) }
            }
            Expr::Map(pairs) => {
                let mut map = IndexMap::new();
                for (k, v) in pairs {
                    let key = self.eval_expr_full(k).to_display();
                    let val = self.eval_expr_full(v);
                    map.insert(key, val);
                }
                Value::Map(map)
            }
         // 変更後
Expr::Array(items) => Value::Array(items.iter().map(|e| self.eval_expr_full(e)).collect()),
Expr::Str(s)    => Value::Str(s.clone()),
Expr::Number(n) => Value::Number(*n),
Expr::Bool(b)   => Value::Bool(*b),
Expr::Var(path) => self.env.get_path_str(path),
        }
    }

    // ── ヘルパー群 ────────────────────────────────────────

    // codegen.rs — impl Codegen 内、「── ヘルパー群 ──」の直後あたりに追加

/// PathSegment列を、eval_expr_full(副作用込みの完全な評価器)を使って
/// 具体的な文字列キー列に解決する。global文・ドット付き代入文のLHSパス解決で使う。
/// （Envは funcs/talks/saori_cache 等のCodegen状態を持たないため、
///  Env単体では choose/saori/log/talk_exists/days_since/format を
///  インデックス式内で使うと常にNullになる制限があった。この関数を経由することで解消する）
pub(crate) fn resolve_path_segments(&mut self, path: &[PathSegment]) -> Vec<String> {
    path.iter().map(|seg| match seg {
        PathSegment::Key(k) => k.clone(),
        PathSegment::Index(expr) => self.eval_expr_full(expr).to_display(),
    }).collect()
}

    
    fn file_reject(&mut self, p: &PathReject) {
        let (level, msg) = match p {
            PathReject::Denied(m) => ("error", m.clone()),
            PathReject::Io(m)     => ("warning", m.clone()),
        };
        append_log!(format!("[file] {}: {}", level, msg));
        self.errors.push((level.to_string(), msg));
    }

    fn file_warn(&mut self, msg: String) {
        append_log!(format!("[file] warning: {}", msg));
        self.errors.push(("warning".to_string(), msg));
    }

    /// 台本のパス文字列を実パスに解決する。セグメント列も返すのは、
    /// 書き込み可否の判定に使うため。
    ///
    /// must_exist=false（書き込み先）のときは、対象がまだ存在せず
    /// canonicalizeできないので、親ディレクトリだけを正規化してから
    /// ファイル名をjoinする。ジャンクションやシンボリックリンク経由の脱出は
    /// この親の正規化で潰れる。
 
fn resolve_file(&self, p: &str, must_exist: bool) -> Result<(PathBuf, Vec<String>), PathReject> {
        let segs = validate_rel_path(p).map_err(PathReject::Denied)?;
        let mut joined = self.home_root.clone();
        for s in &segs { joined.push(s); }

        let full = if must_exist {
            // canonicalizeはリンクも8.3短縮名も解決するので、
            // これ以降のfullは「実体そのもの」を指す
            joined.canonicalize()
                .map_err(|_| PathReject::Io(format!("ファイルが見つかりません: 「{}」", p)))?
        } else {
            let parent = joined.parent()
                .ok_or_else(|| PathReject::Denied(format!("パスが不正です: 「{}」", p)))?;
            let name = joined.file_name()
                .ok_or_else(|| PathReject::Denied(format!("ファイル名がありません: 「{}」", p)))?
                .to_os_string();
            let parent_c = parent.canonicalize()
                .map_err(|_| PathReject::Io(format!("保存先のフォルダがありません: 「{}」", p)))?;
            let full = parent_c.join(name);
            // 末端だけは解決されていないため、ここがリンクだと
            // 書き込みがhome_rootの外へ抜ける
            ensure_not_link(&full, p)?;
            full
        };

        if !full.starts_with(&self.home_root) {
            return Err(PathReject::Denied(
                format!("ゴーストのフォルダの外は指定できません: 「{}」", p)
            ));
        }

        // 書き込み可否は「作者が書いた綴り」ではなく実パスで判定する。
        // 8.3短縮名（CONFIG~1.TOM）やジャンクション経由の遠回り
        // （ghost/master/link/config.toml）で禁止判定をすり抜けるのを防ぐ。
        let checked: Vec<String> = full.strip_prefix(&self.home_root)
            .map_err(|_| PathReject::Denied(format!("パスが不正です: 「{}」", p)))?
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();

        Ok((full, checked))
    }

// 書き込み系の共通前処理。パス解決と書き込み許可の両方を通す。
    fn resolve_writable(&mut self, p: &str) -> Option<PathBuf> {
        let (full, segs) = match self.resolve_file(p, false) {
            Ok(v) => v,
            Err(e) => { self.file_reject(&e); return None; }
        };
        if let Err(msg) = check_writable(&segs) {
            self.file_reject(&PathReject::Denied(msg));
            return None;
        }
        Some(full)
    }
/// cond条件でtalk候補を絞り込む。
/// 条件式の評価中に積まれたエラー（Map未存在キーのnotice等）は、
/// 作者が意図的に書いたnullable参照であることが多く、かつ
/// 「選ばれなかったtalkの条件式」由来なので作者向け情報にならない。
/// そのため、この関数の中でerror。以外を巻き戻す。
/// ※呼び出し側の本体実行より前に必ず巻き戻すこと（後にすると本体のエラーも消える）
fn filter_alive<'t>(&mut self, candidates: &'t [Talk]) -> Vec<(usize, &'t Talk)> {
    let errors_before = self.errors.len();
    let alive: Vec<(usize, &Talk)> = candidates.iter().enumerate()
        .filter(|(_, t)| t.cond.as_ref().map_or(true, |c| self.eval_expr_full(c).as_bool()))
        .collect();
        // cond由来のnotice/warningは捨てるが、errorだけは残す。
    // saoriのロード失敗のような「実行時の値によらない台本の設定ミス」は、
    // たまたま選ばれなかったtalkのcondで起きても作者に伝える必要がある。
    // 捨てる分もDEBUG_LOG時はログに残す（condが効いていないのか
    // エラーが握りつぶされているのかを切り分ける手段が他に無いため）。
    let mut kept: Vec<(String, String)> = self.errors.split_off(errors_before);
    if DEBUG_LOG.load(Ordering::Relaxed) {
        for (_level, _msg) in &kept {
            append_log!(format!("[cond] {}: {}", _level, _msg));
        }
    }
    kept.retain(|(level, _)| level == "error");
    self.errors.append(&mut kept);
    alive
}
    fn call_func(&mut self, name: &str, args: Vec<Value>) -> Value {
        if self.env.call_depth > 100 { append_log!("call depth limit exceeded"); return Value::Null; }
        self.env.call_depth += 1;
        if let Some((params, body)) = self.env.funcs.get(name).cloned() {
            self.env.push_scope();
            for (param, val) in params.iter().zip(args) { self.env.set_local(param, val); }
            let mut out = String::new();
            for s in &body {
                match self.gen_stmt(s, &mut out) {
                    Some(FlowControl::Return(val)) => { self.env.pop_scope(); self.env.call_depth -= 1; return val; }
                    Some(FlowControl::Break) | Some(FlowControl::Continue) => break,
                    None => {}
                }
            }
            self.env.pop_scope();
            self.env.call_depth -= 1;
            Value::Str(out)
        } else { self.env.call_depth -= 1; Value::Null }
    }

    fn call_func_stmt(&mut self, name: &str, args: Vec<Value>, out: &mut String) {
    if self.env.call_depth > 100 { append_log!("call depth limit exceeded"); return; }
    self.env.call_depth += 1;
    if let Some((params, body)) = self.env.funcs.get(name).cloned() {
        self.env.push_scope();
        for (param, val) in params.iter().zip(args) { self.env.set_local(param, val); }
        if let Some(FlowControl::Return(val)) = self.run_stmts(&body, out) {
            out.push_str(&val.to_display());
        }
        self.env.pop_scope();
    }
    self.env.call_depth -= 1;
}

    fn run_stmts(&mut self, stmts: &[Stmt], out: &mut String) -> Option<FlowControl> {
        for s in stmts {
            if let Some(fc) = self.gen_stmt(s, out) { return Some(fc); }
        }
        None
    }

    fn step_for(&mut self, step: &Stmt) {
        if let Stmt::Global(path, op, expr) = step {
            if let [PathSegment::Key(key)] = path.as_slice() {
                let val = self.eval_expr_full(expr);
                self.env.set_var(key, op, val);
                return;
            }
        }
        let mut dummy = String::new();
        self.gen_stmt(step, &mut dummy);
    }

    fn eval_parts(&mut self, parts: &[StrPart]) -> String {
        let mut result = String::new();
        for p in parts {
            let s = match p {
                StrPart::Lit(s) => s.clone(),
                StrPart::Var(path) => {
                    let val = self.env.get_path_str(path).to_display();
                    if val.contains("${") { eval_str_with_vars(&val, &self.env) } else { val }
                }
                StrPart::Expr(e) => self.eval_expr_full(e).to_display(),
            };
            result.push_str(&s);
        }
        result
    }
fn gen_dialogue(&mut self, line: &Line, out: &mut String) {
    let content = self.eval_parts(&line.content);

    match line.character {
        Some(ref chara) => {
            let tag = self.env.characters.get(chara)
                .cloned().unwrap_or_else(|| "\\0".to_string());
            if self.current_scope.as_deref() != Some(tag.as_str()) {
                out.push_str(&tag);
                // 戻ってきたスコープには既に本文があるので、直結させない
                if self.auto_newline && self.spoken_scopes.contains(&tag) {
                    out.push_str("\\n");
                }
                self.current_scope = Some(tag.clone());
            }
            self.spoken_scopes.insert(tag);
            // \sはカレントスコープに効く。必ずタグより後に出す
            if let Some(s) = line.surface {
                out.push_str(&format!("\\s[{}]", s));
            }
        }
        None => {
            if let Some(s) = line.surface {
                out.push_str(&format!("\\s[{}]", s));
            }
            // 台本が\1等でスコープを変えた可能性があるため、追跡をやり直す
            self.current_scope = None;
        }
    }
    out.push_str(&content);
    
   
}
} // impl Codegen


    
// ── テスト ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{program_with_include, ProgramItem, Talk};
    use chumsky::Parser;
    use std::path::PathBuf;
    use indexmap::IndexMap;

    fn make_gen() -> Codegen {
        let mut chars = HashMap::new();
        chars.insert("湊".to_string(),       "\\0".to_string());
        chars.insert("マードック".to_string(), "\\1".to_string());
        Codegen::new(chars, HashMap::new(), PathBuf::from("."))
    }

    fn parse_talks(src: &str) -> Vec<Talk> {
        use crate::parser::preprocess;
        let preprocessed = preprocess(src).expect("preprocess failed");
        let x = program_with_include().parse(&*preprocessed).unwrap();
        x.into_iter().filter_map(|item| if let ProgramItem::Talk(t) = item { Some(t) } else { None }).collect()
    }

    const FIXED_TIME: Option<(i32, u32, u32, u32, u32, u32)> = Some((2026, 1, 1, 12, 0, 0));

    #[test]
    fn test_simple_dialogue() {
        let src = r#"OnBoot => {
    湊: おはようございます。
}"#;
        let talks = parse_talks(src);
        let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\0おはようございます。\\e");
    }

    #[test]
    fn test_empty_map() {
        let src = r#"OnBoot => {
    let d = {}
    global save.count = len(d)
    湊: ${save.count}個。
}"#;
        let talks = parse_talks(src);
        let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\00個。\\e");
    }



    #[test]
    fn test_surface() {
        let src = r#"OnBoot => {
    [0]湊: にっこり。
    [1]湊: あれ。
}"#;
        let talks = parse_talks(src);
        let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
assert_eq!(out, "\\0\\s[0]にっこり。\\n\\s[1]あれ。\\e");
    }

    #[test]
    fn test_global_and_interpolation() {
        let src = r#"
OnBoot => {
    global save.訪問回数 += 1
    湊: ${save.訪問回数}回目だね。
}"#;
        let talks = parse_talks(src);
        let mut cg = make_gen();
        cg.env.globals.insert("save".to_string(), Value::Map({ let mut m = IndexMap::new(); m.insert("訪問回数".to_string(), Value::Number(2.0)); m }));
        let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\03回目だね。\\e");
    }

    #[test]
    fn test_if_else() {
        let src = r#"
OnBoot => {
    if (save.好感度 >= 10) {
        湊: また会えたね。
    } else {
        湊: …おはよう。
    }
}"#;
        let talks = parse_talks(src);
        let mut cg = make_gen();
        cg.env.globals.insert("save".to_string(), Value::Map({ let mut m = IndexMap::new(); m.insert("好感度".to_string(), Value::Number(10.0)); m }));
        let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\0また会えたね。\\e");
    }

    #[test]
    fn test_null_coalesce() {
        let src = r#"OnBoot => {
    湊: ${save.未定義 ?? "初期値"}。
}"#;
        let talks = parse_talks(src);
        let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\0初期値。\\e");
    }

    #[test]
    fn test_builtin_len() {
        let src = r#"OnBoot => {
    湊: ${len("こんにちは")}文字。
}"#;
        let talks = parse_talks(src);
        let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\05文字。\\e");
    }

    #[test]
    fn test_no_chara_dialogue() {
        let src = r#"OnClose => {
    湊: またね。
    \-
}"#;
        let talks = parse_talks(src);
        let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\0またね。\\-\\e");
    }

    #[test]
    fn test_zero_division() {
        let src = r#"OnBoot => {
    湊: ${1 / 0 ?? "ゼロ除算"}。
}"#;
        let talks = parse_talks(src);
        let mut cg = make_gen();
        let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\0ゼロ除算。\\e");
        assert!(cg.errors.iter().any(|(l, _)| l == "warning"));
    }

    #[test]
    fn test_days_since_virtual_time() {
        let src = r#"OnBoot => {
    湊: ${days_since(2026, 1, 1)}日。
}"#;
        let talks = parse_talks(src);
        let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\00日。\\e");
    }
    #[test]
fn test_multiline_dialogue() {
    let src = r#"OnBoot => {
    湊: ほげ
    ふが
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    println!("out: {:?}", out);
    // とりあえずassertなしで中身を確認
    assert!(true);
}
    #[test]
    fn test_multiline_array() {
        let src = r#"OnBoot => {
    let items = [
        "あ",
        "い",
        "う",
    ]
    湊: ${items[1]}。
}"#;
        let talks = parse_talks(src);
        let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\0い。\\e");
    }


#[test]
fn test_dotted_array_index_access() {
    let src = r#"OnBoot => {
    let items = ["あ", "い", "う"]
    湊: ${items.1}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0い\\e");
}

#[test]
fn test_dotted_nested_array_in_map() {
    let src = r#"OnBoot => {
    let save = {list: ["a", "b", "c"]}
    湊: ${save.list.2}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0c\\e");
}

#[test]
fn test_dotted_array_out_of_range_is_null() {
    let src = r#"OnBoot => {
    let items = ["あ"]
    湊: [${items.5 ?? "なし"}]
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0[なし]\\e");
}




    #[test]
    fn test_for_loop() {
        let src = r#"OnBoot => {
    let sum = 0
    for (let i = 0; i < 5; i++) {
        sum += i
    }
    湊: ${sum}
}"#;
        let talks = parse_talks(src);
        let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\010\\e");
    }

    #[test]
    fn test_for_break() {
        let src = r#"OnBoot => {
    let sum = 0
    for (let i = 0; i < 10; i++) {
        if (i == 3) {
            break
        }
        sum += i
    }
    湊: ${sum}
}"#;
        let talks = parse_talks(src);
        let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\03\\e");
    }

    #[test]
    fn test_cond_talk_filter() {
        let src = r#"
OnRandomTalk if(1 == 2) => {
    湊: 選ばれないはず。
}
OnRandomTalk => {
    湊: こちらが選ばれる。
}
"#;
        let talks = parse_talks(src);
        let mut cg = make_gen();
        // gen_event 経由で全候補をフィルタ
        let all: Vec<Talk> = talks.clone();
        let out = cg.gen_event("OnRandomTalk", &all, &HashMap::new(), FIXED_TIME).unwrap();
        assert_eq!(out, "\\0こちらが選ばれる。\\e");
    }
    #[test]
fn test_talk_exists_true() {
    let src = r#"OnBoot => {
    湊: ${talk_exists("OnBoot")}
}"#;
    let talks = parse_talks(src);

    let mut chars = HashMap::new();
    chars.insert("湊".to_string(), "\\0".to_string());
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    talk_map.insert("OnBoot".to_string(), talks.clone());

    let mut cg = Codegen::new(chars, talk_map, PathBuf::from("."));
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0true\\e");
}

#[test]
fn test_talk_exists_false() {
    let src = r#"OnBoot => {
    湊: ${talk_exists("存在しないやつ")}
}"#;
    let talks = parse_talks(src);
    // このテストはtalksが空でOK（"存在しないやつ"が本当に存在しないことを見たいので）
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0false\\e");
}

    #[test]
fn test_call_reference_dotted() {
    let src = r#"
target_talk => {
    湊: 呼ばれた。
}
OnChoiceSelect => {
    call reference.0
}
"#;
    let talks = parse_talks(src);
    let mut chars = HashMap::new();
    chars.insert("湊".to_string(), "\\0".to_string());
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    for t in &talks {
        talk_map.entry(t.event.clone()).or_default().push(t.clone());
    }
    let mut cg = Codegen::new(chars, talk_map, PathBuf::from("."));
    let choice_talk = talks.iter().find(|t| t.event == "OnChoiceSelect").unwrap();
    let mut refs = HashMap::new();
    refs.insert("0".to_string(), "target_talk".to_string());
    let out = cg.gen_talk(choice_talk, &refs, FIXED_TIME);
    assert_eq!(out, "\\0呼ばれた。\\e");
}
#[test]
fn test_not_call_in_or_expr() {
    let src = r#"OnBoot => {
    if (is_null(reference.0) || !talk_exists(reference.0)) {
        湊: ダメ
    } else {
        湊: OK
    }
}"#;
    let talks = parse_talks(src); // パースが通ることを確認するだけでもOK
    assert_eq!(talks.len(), 1);
}

#[test]
fn test_call_func_with_return() {
    let src = r#"OnBoot => {
    func 挨拶する() {
        湊: こんにちは
    }
    call 挨拶する
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0こんにちは\\e");
}

#[test]
fn test_empty_output_records_error() {
    let src = r#"
OnBoot => {
    if (false) {
        湊: 出ない
    }
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let all: Vec<Talk> = talks.clone();
    let result = cg.gen_event("OnBoot", &all, &HashMap::new(), FIXED_TIME);
    assert!(result.is_none());
    assert!(cg.errors.iter().any(|(level, msg)| level == "notice" && msg.contains("出力が空")));
}


#[test]
fn test_call_depth_resets_after_sequential_calls() {
    let src = r#"
target_talk => {
    湊: 呼ばれた
}
OnBoot => {
    for (let i = 0; i < 150; i++) {
        call target_talk
    }
    call target_talk
}
"#;
    let talks = parse_talks(src);
    let mut chars = HashMap::new();
    chars.insert("湊".to_string(), "\\0".to_string());
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    for t in &talks {
        talk_map.entry(t.event.clone()).or_default().push(t.clone());
    }
    let mut cg = Codegen::new(chars, talk_map, PathBuf::from("."));
    let boot_talk = talks.iter().find(|t| t.event == "OnBoot").unwrap();
    let out = cg.gen_talk(boot_talk, &HashMap::new(), FIXED_TIME);

    // 151回 call しているので「呼ばれた」も151回出るはず
    assert_eq!(out.matches("呼ばれた").count(), 151);
}




#[test]
fn test_not_and_precedence_functional() {
    let src = r#"OnBoot => {
    if (!save.flag && save.other) {
        湊: A
    } else {
        湊: B
    }
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    cg.env.globals.insert("save".to_string(), Value::Map({
        let mut m = IndexMap::new();
        m.insert("flag".to_string(), Value::Bool(false));
        m.insert("other".to_string(), Value::Bool(false));
        m
    }));
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);

    // 正しい優先順位: (!flag) && other = true && false = false → else(B)
    // 旧cond_exprのバグ: !(flag && other) = !(false && false) = true → then(A) が誤って選ばれる
    assert_eq!(out, "\\0B\\e");
}


// codegen.rs のテストモジュール（tests）内

#[test]
fn test_dotted_assign_updates_local_not_global() {
    let src = r#"OnBoot => {
    let m = {'a': 1}
    m.a = 2
    湊: ${m.a}
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\02\\e");
    // 修正前バグ: ここでglobals.mに偽物のMapができてしまっていた
    assert!(cg.env.globals.get("m").is_none());
}
#[test]
fn test_not_binds_looser_than_index() {
    let src = r#"OnBoot => {
    let items = [false, true]
    if (!items[0]) {
        湊: A
    } else {
        湊: B
    }
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0A\\e");
}

#[test]
fn test_not_true_literal() {
    let src = r#"OnBoot => {
    if (!true) {
        湊: A
    } else {
        湊: B
    }
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0B\\e");
}


#[test]
fn test_empty_array_is_falsy() {
    let src = r#"OnBoot => {
    let items = []
    if (!items) {
        湊: 空
    } else {
        湊: 中身あり
    }
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0空\\e");
}

#[test]
fn test_nonempty_array_is_truthy() {
    let src = r#"OnBoot => {
    let items = [1, 2]
    if (!items) {
        湊: 空
    } else {
        湊: 中身あり
    }
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0中身あり\\e");
}

#[test]
fn test_empty_map_is_falsy() {
    let src = r#"OnBoot => {
    let m = {}
    if (!m) {
        湊: 空
    } else {
        湊: 中身あり
    }
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0空\\e");
}

#[test]
fn test_nonempty_map_is_truthy() {
    let src = r#"OnBoot => {
    let m = {a: 1}
    if (!m) {
        湊: 空
    } else {
        湊: 中身あり
    }
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0中身あり\\e");
}

#[test]
fn test_double_not() {
    let src = r#"OnBoot => {
    if (!!false) {
        湊: A
    } else {
        湊: B
    }
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0B\\e");
}


#[test]
fn test_global_keyword_still_bypasses_local_scope() {
    // globalキーワードは今回の修正の対象外。挙動が変わっていないことの確認。
    let src = r#"OnBoot => {
    let m = {'a': 1}
    global m.a = 99
    湊: ${m.a}
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    // ローカルmはlet時のまま（globalは別名前空間として上書きするだけ）
    assert_eq!(out, "\\01\\e");
    match cg.env.globals.get("m") {
        Some(Value::Map(m)) => assert_eq!(m["a"].as_number(), 99.0),
        other => panic!("globals.mが更新されているべき: {:?}", other),
    }
}

#[test]
fn test_dotted_assign_without_local_shadow_still_reaches_globals() {
    // save.* のようにローカルに同名変数が無いケースは従来通りglobalsに届くこと
    let src = r#"OnBoot => {
    save.count = 5
    湊: ${save.count}
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\05\\e");
    match cg.env.globals.get("save") {
        Some(Value::Map(m)) => assert_eq!(m["count"].as_number(), 5.0),
        other => panic!("globals.saveが更新されているべき: {:?}", other),
    }
}


// codegen.rs のテストモジュール内

#[test]
fn test_bare_map_key_is_literal_not_variable_lookup() {
    let src = r#"OnBoot => {
    let m = {a: 1, b: 2}
    湊: ${m.a}と${m.b}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\01と2\\e");
}

#[test]
fn test_computed_map_key_still_works_with_bracket_syntax() {
    let src = r#"OnBoot => {
    let k = 'x'
    let m = {[k]: 42}
    湊: ${m.x}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\042\\e");
}

#[test]
fn test_japanese_bare_map_key() {
    let src = r#"OnBoot => {
    let m = {名前: "湊"}
    湊: ${m.名前}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0湊\\e");
}



// codegen.rs のテストモジュール内

#[test]
fn test_choose_unchosen_branch_never_evaluated() {
    let src = r#"OnBoot => {
    global save.calls = 0
    func bump() {
        global save.calls += 1
        return save.calls
    }
    let picked = choose(false, bump(), 99)
    湊: ${picked}/${save.calls}
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    // condがfalseなのでbump()は一度も呼ばれない。save.callsは0のまま。
    assert_eq!(out, "\\099/0\\e");
}

#[test]
fn test_choose_chosen_branch_evaluated_exactly_once() {
    let src = r#"OnBoot => {
    global save.calls = 0
    func bump() {
        global save.calls += 1
        return save.calls
    }
    let picked = choose(true, bump(), 99)
    湊: ${picked}/${save.calls}
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    // condがtrueなのでbump()は正確に1回だけ呼ばれる。
    assert_eq!(out, "\\01/1\\e");
}


#[test]
fn test_global_index_expr_can_use_extended_builtins() {
    let src = r#"OnBoot => {
    global save.items["${format('slot%d', 2)}"] = "剣"
    湊: ${save.items.slot2}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0剣\\e");
}

#[test]
fn test_assign_index_expr_can_use_talk_exists() {
    let src = r#"
target_talk => {
    湊: 中身
}
OnBoot => {
    let m = {}
    m["${choose(talk_exists('target_talk'), 'found', 'missing')}"] = 1
    湊: ${m.found}/${m.missing}
}
"#;
    let talks = parse_talks(src);
    let mut chars = HashMap::new();
    chars.insert("湊".to_string(), "\\0".to_string());
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    for t in &talks {
        talk_map.entry(t.event.clone()).or_default().push(t.clone());
    }
    let mut cg = Codegen::new(chars, talk_map, PathBuf::from("."));
    let boot_talk = talks.iter().find(|t| t.event == "OnBoot").unwrap();
    let out = cg.gen_talk(boot_talk, &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\01/\\e");
}
#[test]
fn test_map_equality_is_structural_not_always_true() {
    let src = r#"OnBoot => {
    let a = {x: 1}
    let b = {x: 2}
    if (a == b) {
        湊: 同じ
    } else {
        湊: 違う
    }
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0違う\\e");
}

#[test]
fn test_map_equality_true_when_contents_match() {
    let src = r#"OnBoot => {
    let a = {x: 1, y: 'あ'}
    let b = {x: 1, y: 'あ'}
    if (a == b) {
        湊: 同じ
    } else {
        湊: 違う
    }
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0同じ\\e");
}

#[test]
fn test_array_of_maps_equality_is_structural() {
    let src = r#"OnBoot => {
    let a = [{x: 1}, {x: 2}]
    let b = [{x: 1}, {x: 9}]
    if (a == b) {
        湊: 同じ
    } else {
        湊: 違う
    }
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0違う\\e");
}


#[test]
fn test_index_of_finds_matching_map_element() {
    let src = r#"OnBoot => {
    let arr = [{x: 1}, {x: 2}, {x: 3}]
    湊: ${index_of(arr, {x: 2})}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\01\\e");
}

#[test]
fn test_index_of_returns_minus_one_when_map_not_found() {
    let src = r#"OnBoot => {
    let arr = [{x: 1}, {x: 2}]
    湊: ${index_of(arr, {x: 99})}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0-1\\e");
}

#[test]
fn test_count_counts_matching_map_elements() {
    let src = r#"OnBoot => {
    let arr = [{x: 1}, {x: 2}, {x: 1}]
    湊: ${count(arr, {x: 1})}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\02\\e");
}

#[test]
fn test_unique_deduplicates_structurally_equal_maps() {
    let src = r#"OnBoot => {
    let arr = [{x: 1}, {x: 1}, {x: 2}]
    let u = unique(arr)
    湊: ${len(u)}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\02\\e");
}

#[test]
fn test_unique_keeps_distinct_maps() {
    let src = r#"OnBoot => {
    let arr = [{x: 1}, {x: 2}, {x: 3}]
    let u = unique(arr)
    湊: ${len(u)}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\03\\e");
}

// 既存の文字列/数値配列での挙動が変わっていないことの回帰確認
#[test]
fn test_index_of_still_works_for_primitives() {
    let src = r#"OnBoot => {
    let arr = ['a', 'b', 'c']
    湊: ${index_of(arr, 'b')}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\01\\e");
}

#[test]
fn test_unique_still_works_for_primitives() {
    let src = r#"OnBoot => {
    let arr = [1, 2, 2, 3, 1]
    let u = unique(arr)
    湊: ${len(u)}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\03\\e");
}

#[test]
fn test_number_string_loose_equality_still_works() {
    // reference.0 のような文字列と数値リテラルの緩い比較（既存挙動）を壊していないことの確認
    let src = r#"OnBoot => {
    if (reference.0 == 1) {
        湊: 一致
    } else {
        湊: 不一致
    }
}"#;
    let talks = parse_talks(src);
    let mut refs = HashMap::new();
    refs.insert("0".to_string(), "1".to_string());
    let out = make_gen().gen_talk(&talks[0], &refs, FIXED_TIME);
    assert_eq!(out, "\\0一致\\e");
}

#[test]
fn test_identifier_starting_with_keyword_evaluates_correctly() {
    let src = r#"OnBoot => {
    let truename = 'たまねぎ'
    let breakfast = 'あさごはん'
    湊: ${truename}と${breakfast}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0たまねぎとあさごはん\\e");
}
#[test]
fn test_identifier_starting_with_keyword_in_expr_position() {
    // 変数名側ではなく、式(atom)として参照される位置で誤爆することを確認する
    let src = r#"OnBoot => {
    let trueflag = 1
    let x = trueflag + 1
    湊: ${x}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\02\\e");
}




#[test]
fn test_for_loop_limit_records_warning() {
    let src = r#"OnBoot => {
    for (let i = 0; i < 999999; i++) {
    }
    湊: おわり
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0おわり\\e");
    assert!(
        cg.errors.iter().any(|(level, msg)| level == "warning" && msg.contains("ループ上限")),
        "ループ上限の警告が記録されていない: {:?}", cg.errors
    );
}

#[test]
fn test_while_loop_limit_records_warning() {
    let src = r#"OnBoot => {
    let x = 0
    while (x >= 0) {
        x += 1
    }
    湊: おわり
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0おわり\\e");
    assert!(
        cg.errors.iter().any(|(level, msg)| level == "warning" && msg.contains("ループ上限")),
        "ループ上限の警告が記録されていない: {:?}", cg.errors
    );
}
#[test]
fn test_foreach_over_limit_array_records_warning() {
    let src = r#"OnBoot => {
    let total = 0
    foreach big_array as v {
        total += 1
    }
    湊: ${total}
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    // Rust側で直接2001要素の配列をglobalsに注入する
    let big: Vec<Value> = (0..2001).map(|i| Value::Number(i as f64)).collect();
    cg.env.globals.insert("big_array".to_string(), Value::Array(big));

    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\02000\\e"); // 2000件で打ち切られるのでtotalは2000
    assert!(
        cg.errors.iter().any(|(level, msg)| level == "warning" && msg.contains("ループ上限")),
        "foreachのループ上限警告が記録されていない: {:?}", cg.errors
    );
}

#[test]
fn test_no_warning_when_loop_finishes_normally() {
    let src = r#"OnBoot => {
    for (let i = 0; i < 10; i++) {
    }
    湊: おわり
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0おわり\\e");
    assert!(
        !cg.errors.iter().any(|(_, msg)| msg.contains("ループ上限")),
        "正常終了したループなのに警告が出ている: {:?}", cg.errors
    );
}



#[test]
fn test_format_d_zero_pad_negative_number() {
    let src = r#"OnBoot => {
    湊: ${format('%05d', -5)}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0-0005\\e");
}

#[test]
fn test_format_d_zero_pad_positive_number() {
    let src = r#"OnBoot => {
    湊: ${format('%05d', 42)}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\000042\\e");
}

#[test]
fn test_format_f_respects_width() {
    let src = r#"OnBoot => {
    湊: [${format('%8.2f', 3.14159)}]
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    // "3.14" は4文字なので、幅8で右寄せすると前に半角スペース4個
    assert_eq!(out, "\\0[    3.14]\\e");
}

#[test]
fn test_format_f_zero_pad_with_width() {
    let src = r#"OnBoot => {
    湊: ${format('%08.2f', 3.14159)}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\000003.14\\e");   // ゼロ5個（タグの1個 + パディング4個）
}

#[test]
fn test_format_s_respects_width() {
    let src = r#"OnBoot => {
    湊: [${format('%10s', '湊')}]
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    // "湊" は1文字なので、幅10で右寄せすると前に半角スペース9個
    assert_eq!(out, "\\0[         湊]\\e");
}

#[test]
fn test_format_no_width_specifier_unchanged() {
    // width指定なしの既存挙動（回帰確認）
    let src = r#"OnBoot => {
    湊: ${format('%d/%s', 7, 'あ')}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\07/あ\\e");
}

#[test]
fn test_format_huge_width_is_clamped_instead_of_allocating_unbounded() {
    // 辞書スクリプトが桁数の大きいwidthを指定しても、数十億文字の確保を
    // 試みてabortすることなく、FORMAT_MAX_WIDTHでクランプされること
    let src = r#"OnBoot => {
    湊: ${format('%9999999999999999999999s', 'x')}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    // "\0" + パディング + "x" + "\e" のうち、パディング長がFORMAT_MAX_WIDTH-1以下に収まっていること
    assert!(out.len() < 2000, "widthがクランプされずに巨大な文字列になっている: len={}", out.len());
}

#[test]
fn test_format_huge_precision_is_clamped() {
    let src = r#"OnBoot => {
    湊: ${format('%.9999999999999999999999f', 1.5)}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert!(out.len() < 2000, "precisionがクランプされずに巨大な文字列になっている: len={}", out.len());
}

#[test]
fn test_to_hex_huge_digits_is_clamped() {
    // 辞書スクリプトがto_hexの桁数指定に巨大な値を渡しても、
    // FORMAT_MAX_WIDTHでクランプされOOM abortに至らないこと
    let src = r#"OnBoot => {
    湊: ${to_hex(1, 99999999999999999999)}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert!(out.len() < 2000, "digitsがクランプされずに巨大な文字列になっている: len={}", out.len());
}

#[test]
fn test_to_hex_normal_digits_unchanged() {
    let src = r#"OnBoot => {
    湊: ${to_hex(255, 4)}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\000ff\\e");
}



#[test]
fn test_call_stmt_respects_cond_filter() {
    let src = r#"
target_talk if(false) => {
    湊: 選ばれないはず
}
target_talk => {
    湊: こっちが選ばれる
}
OnBoot => {
    call target_talk
}
"#;
    let talks = parse_talks(src);
    let mut chars = HashMap::new();
    chars.insert("湊".to_string(), "\\0".to_string());
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    for t in &talks {
        talk_map.entry(t.event.clone()).or_default().push(t.clone());
    }
    let mut cg = Codegen::new(chars, talk_map, PathBuf::from("."));
    let boot_talk = talks.iter().find(|t| t.event == "OnBoot").unwrap();
    let out = cg.gen_talk(boot_talk, &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0こっちが選ばれる\\e");
}

#[test]
fn test_call_stmt_all_cond_false_produces_no_output() {
    let src = r#"
target_talk if(false) => {
    湊: 出ないはず
}
OnBoot => {
    call target_talk
    湊: 後続は実行される
}
"#;
    let talks = parse_talks(src);
    let mut chars = HashMap::new();
    chars.insert("湊".to_string(), "\\0".to_string());
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    for t in &talks {
        talk_map.entry(t.event.clone()).or_default().push(t.clone());
    }
    let mut cg = Codegen::new(chars, talk_map, PathBuf::from("."));
    let boot_talk = talks.iter().find(|t| t.event == "OnBoot").unwrap();
    let out = cg.gen_talk(boot_talk, &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0後続は実行される\\e");
}

#[test]
fn test_expr_talk_call_respects_cond_filter() {
    let src = r#"
target_talk if(false) => {
    湊: 選ばれないはず
}
target_talk => {
   return '中身'
}
OnBoot => {
    湊: ${target_talk()}
}
"#;
    let talks = parse_talks(src);
    let mut chars = HashMap::new();
    chars.insert("湊".to_string(), "\\0".to_string());
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    for t in &talks {
        talk_map.entry(t.event.clone()).or_default().push(t.clone());
    }
    let mut cg = Codegen::new(chars, talk_map, PathBuf::from("."));
    let boot_talk = talks.iter().find(|t| t.event == "OnBoot").unwrap();
    let out = cg.gen_talk(boot_talk, &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0中身\\e");
}


#[test]
fn test_for_loop_exactly_at_limit_no_warning() {
    let src = format!(r#"OnBoot => {{
    for (let i = 0; i < {}; i++) {{
    }}
    湊: おわり
}}"#, LOOP_LIMIT);
    let talks = parse_talks(&src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0おわり\\e");
    assert!(
        !cg.errors.iter().any(|(_, msg)| msg.contains("ループ上限")),
        "ちょうど上限回数で終わる正常なループなのに警告が出ている: {:?}", cg.errors
    );
}


#[test]
fn test_call_depth_resets_at_start_of_gen_event() {
    let src = r#"OnBoot => {
    湊: おわり
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    cg.env.call_depth = 55; // 前回イベントでパニックして残ったかのような状態を模擬
    let all: Vec<Talk> = talks.clone();
    let out = cg.gen_event("OnBoot", &all, &HashMap::new(), FIXED_TIME).unwrap();
    assert_eq!(out, "\\0おわり\\e");
    assert_eq!(cg.env.call_depth, 0);
}


    #[test]
fn test_errors_reset_at_start_of_gen_event() {
    let src = r#"OnBoot => {
    湊: おわり
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    // 前回イベントがパニックして残ったかのようなエラーを模擬
    cg.errors.push(("warning".to_string(), "前回の残骸".to_string()));

    let all: Vec<Talk> = talks.clone();
    let out = cg.gen_event("OnBoot", &all, &HashMap::new(), FIXED_TIME).unwrap();

    assert_eq!(out, "\\0おわり\\e");
    assert!(
        !cg.errors.iter().any(|(_, m)| m.contains("前回の残骸")),
        "前回イベントのエラーが持ち越されている: {:?}", cg.errors
    );
}


#[test]
fn test_get_array_out_of_range_returns_default_no_warning() {
    let src = r#"OnBoot => {
    let items = []
    湊: ${get(items, 0, 'なし')}
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0なし\\e");
    assert!(cg.errors.is_empty(), "getは警告を出さないはず: {:?}", cg.errors);
}

#[test]
fn test_get_array_in_range_returns_value() {
    let src = r#"OnBoot => {
    let items = [10, 20]
    湊: ${get(items, 1, 0)}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\020\\e");
}
#[test]
fn test_cond_saori_load_error_is_kept() {
    // cond評価中のnoticeは捨てるが、saoriのロード失敗（error）は残す。
    // 存在しないDLLを指定すればload時点でErrになるので、
    // DLLを用意せずに本物の経路を通せる。
    let src = r#"
target_talk if(saori('存在しない.dll')) => {
    湊: 出ないはず
}
target_talk => {
    湊: こっち
}
OnBoot => {
    call target_talk
}
"#;
    let talks = parse_talks(src);
    let mut chars = HashMap::new();
    chars.insert("湊".to_string(), "\\0".to_string());
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    for t in &talks {
        talk_map.entry(t.event.clone()).or_default().push(t.clone());
    }
    let mut cg = Codegen::new(chars, talk_map, PathBuf::from("."));

    let boot_talk = talks.iter().find(|t| t.event == "OnBoot").unwrap();
    let out = cg.gen_talk(boot_talk, &HashMap::new(), FIXED_TIME);

    assert_eq!(out, "\\0こっち\\e");
    assert!(
        cg.errors.iter().any(|(l, _)| l == "error"),
        "cond内のsaoriロード失敗が握りつぶされている: {:?}", cg.errors
    );
}



#[test]
fn test_saori_absolute_path_is_denied() {
    let src = r#"OnBoot => {
    湊: [${saori('C:\\evil.dll')}]
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0[]\\e");
    assert!(
        cg.errors.iter().any(|(l, m)| l == "error" && m.contains("絶対パス") || l == "error" && m.contains("ドライブ")),
        "絶対パスのDLLが拒否されていない: {:?}", cg.errors
    );
}

#[test]
fn test_saori_parent_traversal_is_denied() {
    let src = r#"OnBoot => {
    湊: [${saori('../../evil.dll')}]
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0[]\\e");
    assert!(
        cg.errors.iter().any(|(l, m)| l == "error" && m.contains("..")),
        "親ディレクトリ参照が拒否されていない: {:?}", cg.errors
    );
}

// ── 外部呼び出し予算（get_property/saoriのループ複利フリーズ対策）────

#[test]
fn test_external_call_budget_allows_up_to_limit_then_blocks() {
    // MAX_EXTERNAL_CALLS_PER_EVENT回までは予算内(true)、
    // それを超えると予算切れ(false)になることを直接確認する。
    // EnvはCodegenインスタンスごとに独立しているため、
    // 他のテストと並行実行されても影響し合わない。
    let env = Env::new(HashMap::new());
    for i in 0..MAX_EXTERNAL_CALLS_PER_EVENT {
        assert!(consume_external_call_budget(&env), "呼び出し{}回目は予算内のはず", i + 1);
    }
    assert!(!consume_external_call_budget(&env), "上限到達後はfalseになるはず");
    assert!(!consume_external_call_budget(&env), "予算切れ後は呼ぶたびfalseのままのはず");
}

#[test]
fn test_external_call_budget_resets_with_runtime_state() {
    // gen_event開始時に呼ばれるreset_runtime_stateで予算が復活することの確認。
    let mut env = Env::new(HashMap::new());
    for _ in 0..MAX_EXTERNAL_CALLS_PER_EVENT {
        consume_external_call_budget(&env);
    }
    assert!(!consume_external_call_budget(&env));

    env.reset_runtime_state();
    assert!(consume_external_call_budget(&env), "reset後は予算が復活しているはず");
}

#[test]
fn test_saori_loop_calls_are_capped_per_event() {
    // whileループでSAORIを繰り返し呼んでも、実際にload()を試みる
    // （=STATEロックを長時間保持しうる）回数はMAX_EXTERNAL_CALLS_PER_EVENTで
    // 頭打ちになり、それ以降は「予算切れ」の警告に切り替わることを確認する。
    // 存在しないDLL名を使うことで、本物のload失敗経路（1回ごとにerrorを積む）を
    // そのまま通しつつテストする。
    let loop_count = MAX_EXTERNAL_CALLS_PER_EVENT + 20;
    let src = format!(
        r#"OnBoot => {{
    let i = 0
    while(i < {loop_count}) {{
        saori('存在しない.dll')
        i += 1
    }}
}}"#
    );
    let talks = parse_talks(&src);
    let mut cg = make_gen();
    cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);

    let load_error_count = cg.errors.iter().filter(|(l, _)| l == "error").count();
    let budget_warning_count = cg.errors.iter()
        .filter(|(l, m)| l == "warning" && m.contains("外部呼び出し上限"))
        .count();

    assert_eq!(
        load_error_count, MAX_EXTERNAL_CALLS_PER_EVENT,
        "load失敗のerrorがMAX_EXTERNAL_CALLS_PER_EVENTを超えて発生している（ループが予算で頭打ちになっていない）: {:?}", cg.errors
    );
    assert_eq!(
        budget_warning_count, loop_count - MAX_EXTERNAL_CALLS_PER_EVENT,
        "予算切れ警告の件数が想定と異なる: {:?}", cg.errors
    );
}

#[test]
fn test_get_property_loop_is_capped_per_event() {
    // whileループでget_propertyを繰り返し呼んでも、Env側のカウンタが
    // 呼び出し試行の総数を正しく記録することを確認する
    // （実際にSSTPへ接続を試みるのはこのうちMAX_EXTERNAL_CALLS_PER_EVENT回までで、
    //  それ以降はconsume_external_call_budgetがfalseを返して即座に空文字を返す）。
    let loop_count = MAX_EXTERNAL_CALLS_PER_EVENT + 15;
    let src = format!(
        r#"OnBoot => {{
    let i = 0
    while(i < {loop_count}) {{
        get_property('name')
        i += 1
    }}
}}"#
    );
    let talks = parse_talks(&src);
    let mut cg = make_gen();
    cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);

    assert_eq!(
        cg.env.external_call_count.get(), loop_count,
        "呼び出し試行のカウントがループ回数と一致しない"
    );
}

#[test]
fn test_get_map_missing_key_returns_default() {
    let src = r#"OnBoot => {
    let m = {a: 1}
    湊: ${get(m, 'b', 'なし')}
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let out = cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0なし\\e");
    assert!(cg.errors.is_empty());
}

#[test]
fn test_get_no_default_returns_null() {
    let src = r#"OnBoot => {
    let items = []
    湊: [${get(items, 0)}]
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0[]\\e");
}
#[test]
fn test_days_between_positive() {
    let src = r#"OnBoot => {
    湊: ${days_between(2026, 1, 1, 2026, 1, 11)}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\010\\e");
}

#[test]
fn test_days_between_negative_when_reversed() {
    let src = r#"OnBoot => {
    湊: ${days_between(2026, 1, 11, 2026, 1, 1)}
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0-10\\e");
}

#[test]
fn test_days_between_invalid_date_returns_null() {
    let src = r#"OnBoot => {
    湊: [${days_between(2026, 13, 1, 2026, 1, 1) ?? "無効"}]
}"#;
    let talks = parse_talks(src);
    let out = make_gen().gen_talk(&talks[0], &HashMap::new(), FIXED_TIME);
    assert_eq!(out, "\\0[無効]\\e");
}

#[test]
fn test_locals_reset_at_start_of_gen_event() {
    let src = r#"OnBoot => {
    湊: ${save.x}
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    cg.env.globals.insert("save".to_string(), Value::Map({
        let mut m = IndexMap::new();
        m.insert("x".to_string(), Value::Str("グローバル".to_string()));
        m
    }));

    // 前回イベントがパニックして残ったかのようなローカルスコープを模擬
    cg.env.push_scope();
    cg.env.set_local("save", Value::Map({
        let mut m = IndexMap::new();
        m.insert("x".to_string(), Value::Str("残骸".to_string()));
        m
    }));

    let all: Vec<Talk> = talks.clone();
    let out = cg.gen_event("OnBoot", &all, &HashMap::new(), FIXED_TIME).unwrap();

    // 残骸のローカルがglobalsをシャドウしていないこと
    assert_eq!(out, "\\0グローバル\\e");
    // スコープが積み上がっていないこと（ベース1枚だけ）
    assert_eq!(cg.env.locals.len(), 1, "ローカルスコープが残っている");
}

#[test]
fn test_scope_depth_does_not_grow_across_events() {
    let src = r#"OnBoot => {
    湊: おわり
}"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let all: Vec<Talk> = talks.clone();
    for _ in 0..10 {
        cg.gen_event("OnBoot", &all, &HashMap::new(), FIXED_TIME).unwrap();
    }
    assert_eq!(cg.env.locals.len(), 1);
}

#[test]
fn test_call_stmt_cond_errors_are_discarded() {
    let src = r#"
target_talk if(save.存在しないキー) => {
    湊: 出ないはず
}
target_talk => {
    湊: こっち
}
OnBoot => {
    call target_talk
}
"#;
    let talks = parse_talks(src);
    let mut chars = HashMap::new();
    chars.insert("湊".to_string(), "\\0".to_string());
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    for t in &talks {
        talk_map.entry(t.event.clone()).or_default().push(t.clone());
    }
    let mut cg = Codegen::new(chars, talk_map, PathBuf::from("."));
    cg.env.globals.insert("save".to_string(), Value::Map(IndexMap::new()));

    let boot_talk = talks.iter().find(|t| t.event == "OnBoot").unwrap();
    let out = cg.gen_talk(boot_talk, &HashMap::new(), FIXED_TIME);

    assert_eq!(out, "\\0こっち\\e");
    assert!(
        cg.errors.is_empty(),
        "cond評価の副作用エラーが残っている: {:?}", cg.errors
    );
}

#[test]
fn test_call_stmt_body_errors_are_kept() {
    let src = r#"
target_talk if(save.存在しないキー) => {
    湊: 出ないはず
}
target_talk => {
    湊: ${1 / 0 ?? "ゼロ除算"}
}
OnBoot => {
    call target_talk
}
"#;
    let talks = parse_talks(src);
    let mut chars = HashMap::new();
    chars.insert("湊".to_string(), "\\0".to_string());
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    for t in &talks {
        talk_map.entry(t.event.clone()).or_default().push(t.clone());
    }
    let mut cg = Codegen::new(chars, talk_map, PathBuf::from("."));
    cg.env.globals.insert("save".to_string(), Value::Map(IndexMap::new()));

    let boot_talk = talks.iter().find(|t| t.event == "OnBoot").unwrap();
    let out = cg.gen_talk(boot_talk, &HashMap::new(), FIXED_TIME);

    assert_eq!(out, "\\0ゼロ除算\\e");
    assert!(
        cg.errors.iter().any(|(l, m)| l == "warning" && m.contains("ゼロ除算")),
        "本体実行中のエラーまで捨てられている: {:?}", cg.errors
    );
}

        #[test]
fn test_call_stmt_all_cond_false_records_notice() {
    let src = r#"
target_talk if(false) => {
    湊: 出ないはず
}
OnBoot => {
    call target_talk
    湊: 後続は実行される
}
"#;
    let talks = parse_talks(src);
    let mut chars = HashMap::new();
    chars.insert("湊".to_string(), "\\0".to_string());
    let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
    for t in &talks {
        talk_map.entry(t.event.clone()).or_default().push(t.clone());
    }
    let mut cg = Codegen::new(chars, talk_map, PathBuf::from("."));
    let boot_talk = talks.iter().find(|t| t.event == "OnBoot").unwrap();
    let out = cg.gen_talk(boot_talk, &HashMap::new(), FIXED_TIME);

    assert_eq!(out, "\\0後続は実行される\\e");
    assert!(
        cg.errors.iter().any(|(l, m)| l == "notice" && m.contains("target_talk")),
        "call全滅のnoticeが記録されていない: {:?}", cg.errors
    );
}

#[test]
fn test_gen_event_all_cond_false_stays_silent() {
    // gen_event（SSPからのイベント）の全滅は正常系なので、
    // noticeを出さず無言で204のままであることの確認（縮退防止）
    let src = r#"
OnRandomTalk if(false) => {
    湊: 出ないはず
}
"#;
    let talks = parse_talks(src);
    let mut cg = make_gen();
    let all: Vec<Talk> = talks.clone();
    let result = cg.gen_event("OnRandomTalk", &all, &HashMap::new(), FIXED_TIME);

    assert!(result.is_none());
    assert!(
        cg.errors.is_empty(),
        "イベント全滅で余計なnoticeが出ている: {:?}", cg.errors
    );
}


    fn make_ghost_home() -> (tempfile::TempDir, PathBuf) {
        let home = tempfile::tempdir().expect("tempdir作成失敗");
        let master = home.path().join("ghost").join("master");
        std::fs::create_dir_all(&master).expect("master作成失敗");
        (home, master)
    }

    fn make_file_gen(master: &std::path::Path) -> Codegen {
        let mut chars = HashMap::new();
        chars.insert("湊".to_string(), "\\0".to_string());
        Codegen::new(chars, HashMap::new(), master.to_path_buf())
    }

    fn run_in(cg: &mut Codegen, src: &str) -> String {
        let talks = parse_talks(src);
        cg.gen_talk(&talks[0], &HashMap::new(), FIXED_TIME)
    }


    #[test]
    fn test_symlink_target_outside_home_is_denied() {
        let (_home, master) = make_ghost_home();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("外.txt");
        std::fs::write(&target, "元").unwrap();

        // 開発者モードでないと権限エラーになる。
        // 環境依存なので、作れなかったときは検証せずに抜ける。
        if std::os::windows::fs::symlink_file(&target, master.join("link.txt")).is_err() {
            eprintln!("シンボリックリンクを作成できないためスキップ（開発者モードを確認）");
            return;
        }

        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_write('ghost/master/link.txt', '書き換え')}
}"#);
        assert_eq!(out, "\\0false\\e");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "元");
    }
    #[test]
    fn test_file_write_then_read_roundtrip() {
        let (_home, master) = make_ghost_home();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    let ok = file_write('ghost/master/memo.txt', 'こんにちは')
    湊: ${ok}/${file_read('ghost/master/memo.txt')}
}"#);
        assert_eq!(out, "\\0true/こんにちは\\e");
        assert!(cg.errors.is_empty(), "{:?}", cg.errors);
    }

    #[test]
    fn test_file_write_leaves_no_tmp() {
        let (_home, master) = make_ghost_home();
        let mut cg = make_file_gen(&master);
        run_in(&mut cg, r#"OnBoot => {
    let ok = file_write('ghost/master/memo.txt', 'x')
    湊: ${ok}
}"#);
        assert!(!master.join("memo.txt.minato.tmp").exists(), "tmpが残っている");
    }

    #[test]
    fn test_file_write_to_config_is_denied_as_error() {
        let (_home, master) = make_ghost_home();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_write('ghost/master/config.toml', 'x')}
}"#);
        assert_eq!(out, "\\0false\\e");
        assert!(cg.errors.iter().any(|(l, _)| l == "error"), "{:?}", cg.errors);
        assert!(!master.join("config.toml").exists(), "禁止パスに書けてしまっている");
    }

    #[test]
    fn test_file_write_to_talks_is_denied() {
        let (_home, master) = make_ghost_home();
        std::fs::create_dir_all(master.join("talks")).unwrap();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_write('ghost/master/talks/main.mnt', 'x')}
}"#);
        assert_eq!(out, "\\0false\\e");
        assert!(!master.join("talks").join("main.mnt").exists());
    }

    #[test]
    fn test_file_write_to_minato_log_is_denied() {
        let (_home, master) = make_ghost_home();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_write('ghost/master/minato_load.log', 'x')}
}"#);
        assert_eq!(out, "\\0false\\e");
        assert!(!master.join("minato_load.log").exists());
    }

    #[test]
    fn test_file_write_outside_master_is_denied() {
        let (home, master) = make_ghost_home();
        std::fs::create_dir_all(home.path().join("shell").join("master")).unwrap();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_write('shell/master/menu_background.png', 'x')}
}"#);
        assert_eq!(out, "\\0false\\e");
        assert!(cg.errors.iter().any(|(l, _)| l == "error"));
    }

    #[test]
    fn test_file_read_reaches_whole_home() {
        let (home, master) = make_ghost_home();
        std::fs::create_dir_all(home.path().join("shell").join("master")).unwrap();
        std::fs::write(home.path().join("shell").join("master").join("descript.txt"), "charset,UTF-8").unwrap();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_read('shell/master/descript.txt')}
}"#);
        assert_eq!(out, "\\0charset,UTF-8\\e");
    }

    #[test]
    fn test_file_read_config_is_allowed() {
        let (_home, master) = make_ghost_home();
        std::fs::write(master.join("config.toml"), "ok").unwrap();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_read('ghost/master/config.toml')}
}"#);
        assert_eq!(out, "\\0ok\\e");
    }

    #[test]
    fn test_parent_traversal_is_denied() {
        let (_home, master) = make_ghost_home();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: [${file_read('../../../secret.txt')}]
}"#);
        assert_eq!(out, "\\0[]\\e");
        assert!(cg.errors.iter().any(|(l, m)| l == "error" && m.contains("..")), "{:?}", cg.errors);
    }

    #[test]
    fn test_absolute_path_is_denied() {
        let (_home, master) = make_ghost_home();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: [${file_read('C:\\Windows\\System32\\drivers\\etc\\hosts')}]
}"#);
        assert_eq!(out, "\\0[]\\e");
        assert!(cg.errors.iter().any(|(l, _)| l == "error"));
    }

    #[test]
    fn test_reserved_name_is_denied() {
        let (_home, master) = make_ghost_home();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_write('ghost/master/NUL.txt', 'x')}
}"#);
        assert_eq!(out, "\\0false\\e");
    }

    #[test]
    fn test_file_read_missing_returns_null_with_warning() {
        let (_home, master) = make_ghost_home();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_read('ghost/master/ない.txt') ?? 'なし'}
}"#);
        assert_eq!(out, "\\0なし\\e");
        assert!(cg.errors.iter().any(|(l, _)| l == "warning"), "{:?}", cg.errors);
        assert!(!cg.errors.iter().any(|(l, _)| l == "error"), "存在しないだけでerrorにしている");
    }

    #[test]
    fn test_file_write_missing_parent_fails() {
        let (_home, master) = make_ghost_home();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_write('ghost/master/ないフォルダ/x.txt', 'x')}
}"#);
        assert_eq!(out, "\\0false\\e");
        assert!(!master.join("ないフォルダ").exists(), "勝手にフォルダを作っている");
    }

    #[test]
    fn test_file_append_accumulates() {
        let (_home, master) = make_ghost_home();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    file_append('ghost/master/log.txt', 'あ')
    file_append('ghost/master/log.txt', 'い')
    湊: ${file_read('ghost/master/log.txt')}
}"#);
        assert_eq!(out, "\\0あい\\e");
    }

    #[test]
    fn test_file_move_refuses_existing_dest_by_default() {
        let (_home, master) = make_ghost_home();
        std::fs::write(master.join("a.txt"), "A").unwrap();
        std::fs::write(master.join("b.txt"), "B").unwrap();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_move('ghost/master/a.txt', 'ghost/master/b.txt')}
}"#);
        assert_eq!(out, "\\0false\\e");
        assert_eq!(std::fs::read_to_string(master.join("b.txt")).unwrap(), "B", "黙って上書きされている");
        assert!(master.join("a.txt").exists(), "移動元が消えている");
    }

    #[test]
    fn test_file_move_overwrites_when_explicit() {
        let (_home, master) = make_ghost_home();
        std::fs::write(master.join("a.txt"), "A").unwrap();
        std::fs::write(master.join("b.txt"), "B").unwrap();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_move('ghost/master/a.txt', 'ghost/master/b.txt', true)}
}"#);
        assert_eq!(out, "\\0true\\e");
        assert_eq!(std::fs::read_to_string(master.join("b.txt")).unwrap(), "A");
        assert!(!master.join("a.txt").exists());
    }

    #[test]
    fn test_file_move_into_talks_is_denied() {
        let (_home, master) = make_ghost_home();
        std::fs::create_dir_all(master.join("talks")).unwrap();
        std::fs::write(master.join("a.mnt"), "OnBoot => {}").unwrap();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_move('ghost/master/a.mnt', 'ghost/master/talks/a.mnt')}
}"#);
        assert_eq!(out, "\\0false\\e");
        assert!(!master.join("talks").join("a.mnt").exists());
    }
#[test]
fn test_file_move_same_file_case_difference_is_allowed() {
    let (_home, master) = make_ghost_home();
    std::fs::write(master.join("a.txt"), "A").unwrap();
    let mut cg = make_file_gen(&master);
    let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_move('ghost/master/a.txt', 'ghost/master/A.TXT')}
}"#);
    assert_eq!(out, "\\0true\\e");
}

    #[test]
    fn test_sjis_roundtrip() {
        let (_home, master) = make_ghost_home();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    file_write('ghost/master/sj.txt', '日本語', 'sjis')
    湊: ${file_read('ghost/master/sj.txt', 'sjis')}
}"#);
        assert_eq!(out, "\\0日本語\\e");
    }

    #[test]
    fn test_sjis_file_read_as_utf8_returns_null() {
        let (_home, master) = make_ghost_home();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    file_write('ghost/master/sj.txt', '日本語', 'sjis')
    湊: ${file_read('ghost/master/sj.txt') ?? '読めない'}
}"#);
        assert_eq!(out, "\\0読めない\\e");
    }

    #[test]
    fn test_oversized_file_returns_null_not_truncated() {
        let (_home, master) = make_ghost_home();
        let big = "a".repeat((FILE_READ_LIMIT as usize) + 10);
        std::fs::write(master.join("big.txt"), &big).unwrap();
        let mut cg = make_file_gen(&master);
        let out = run_in(&mut cg, r#"OnBoot => {
    湊: ${file_read('ghost/master/big.txt') ?? '大きすぎ'}
}"#);
        assert_eq!(out, "\\0大きすぎ\\e");
    }

    #[test]
    fn test_home_falls_back_when_not_ghost_master() {
        // ghost/master 構成でないディレクトリを渡したとき、
        // 親を辿ってサンドボックスが広がっていないこと
        let dir = tempfile::tempdir().unwrap();
        let cg = make_file_gen(dir.path());
        let canonical = dir.path().canonicalize().unwrap();
        assert_eq!(cg.home_root, canonical, "ghost/master構成でないのに親へ上っている");
    }

    #[test]
    fn test_sandbox_violation_survives_cond_filter() {
        // filter_aliveはcond由来のnotice/warningを捨てるが、
        // サンドボックス違反はerrorなので作者に届く
        let (_home, master) = make_ghost_home();
        let src = r#"
target_talk if(file_write('../../外.txt', 'x')) => {
    湊: 出ないはず
}
target_talk => {
    湊: こっち
}
OnBoot => {
    call target_talk
}
"#;
        let talks = parse_talks(src);
        let mut chars = HashMap::new();
        chars.insert("湊".to_string(), "\\0".to_string());
        let mut talk_map: HashMap<String, Vec<Talk>> = HashMap::new();
        for t in &talks {
            talk_map.entry(t.event.clone()).or_default().push(t.clone());
        }
        let mut cg = Codegen::new(chars, talk_map, master.clone());
        let boot = talks.iter().find(|t| t.event == "OnBoot").unwrap();
        let out = cg.gen_talk(boot, &HashMap::new(), FIXED_TIME);
        assert_eq!(out, "\\0こっち\\e");
        assert!(cg.errors.iter().any(|(l, _)| l == "error"), "{:?}", cg.errors);
    }

}

#[cfg(test)]
mod set_path_tests {
    use super::*;
    use crate::parser::{AssignOp, PathSegment, Expr};
    use indexmap::IndexMap;

    fn make_env() -> Env { Env::new(HashMap::new()) }

    #[test]
    fn test_set_path_simple() {
        let mut env = make_env();
        env.set_path(&[PathSegment::Key("x".to_string())], &AssignOp::Set, Value::Number(42.0));
        assert_eq!(env.globals["x"].as_number(), 42.0);
    }

    #[test]
    fn test_set_path_creates_map_from_null() {
        let mut env = make_env();
        env.set_path(&[PathSegment::Key("save".to_string()), PathSegment::Key("好感度".to_string())], &AssignOp::Set, Value::Number(10.0));
        let save = match env.globals.get("save") { Some(Value::Map(m)) => m.clone(), _ => panic!() };
        assert_eq!(save["好感度"].as_number(), 10.0);
    }

    #[test]
    fn test_set_path_extends_existing_map() {
        let mut env = make_env();
        env.set_path(&[PathSegment::Key("save".to_string()), PathSegment::Key("a".to_string())], &AssignOp::Set, Value::Number(1.0));
        env.set_path(&[PathSegment::Key("save".to_string()), PathSegment::Key("b".to_string())], &AssignOp::Set, Value::Number(2.0));
        let save = match env.globals.get("save") { Some(Value::Map(m)) => m.clone(), _ => panic!() };
        assert_eq!(save["a"].as_number(), 1.0);
        assert_eq!(save["b"].as_number(), 2.0);
    }

    #[test]
    fn test_set_path_add_assign() {
        let mut env = make_env();
        env.set_path(&[PathSegment::Key("save".to_string()), PathSegment::Key("訪問回数".to_string())], &AssignOp::Set, Value::Number(5.0));
        env.set_path(&[PathSegment::Key("save".to_string()), PathSegment::Key("訪問回数".to_string())], &AssignOp::Add, Value::Number(1.0));
        let save = match env.globals.get("save") { Some(Value::Map(m)) => m.clone(), _ => panic!() };
        assert_eq!(save["訪問回数"].as_number(), 6.0);
    }
    
    #[test]
    fn test_set_path_set_if_null() {
        let mut env = make_env();
        env.set_path(&[PathSegment::Key("save".to_string()), PathSegment::Key("初期値".to_string())], &AssignOp::SetIfNull, Value::Number(0.0));
        env.set_path(&[PathSegment::Key("save".to_string()), PathSegment::Key("初期値".to_string())], &AssignOp::SetIfNull, Value::Number(99.0));
        let save = match env.globals.get("save") { Some(Value::Map(m)) => m.clone(), _ => panic!() };
        assert_eq!(save["初期値"].as_number(), 0.0);
    }

    #[test]
    fn test_set_path_array_index_from_null() {
        let mut env = make_env();
        env.set_path(&[PathSegment::Key("save".to_string()), PathSegment::Key("list".to_string()), PathSegment::Index(Expr::Number(0.0))], &AssignOp::Set, Value::Str("x".to_string()));
        let save = match env.globals.get("save") { Some(Value::Map(m)) => m.clone(), _ => panic!() };
        match save.get("list") {
            Some(Value::Map(m)) => assert_eq!(m["0"].to_display(), "x"),
            Some(Value::Array(a)) => assert_eq!(a[0].to_display(), "x"),
            other => panic!("予期しない型: {:?}", other),
        }
    }

    #[test]
    fn test_set_path_array_index_existing() {
        let mut env = make_env();
        let arr = Value::Array(vec![Value::Str("a".to_string()), Value::Str("b".to_string()), Value::Str("c".to_string())]);
        let mut m = IndexMap::new(); m.insert("list".to_string(), arr);
        env.globals.insert("save".to_string(), Value::Map(m));
        env.set_path(&[PathSegment::Key("save".to_string()), PathSegment::Key("list".to_string()), PathSegment::Index(Expr::Number(1.0))], &AssignOp::Set, Value::Str("X".to_string()));
        let save = match env.globals.get("save") { Some(Value::Map(m)) => m.clone(), _ => panic!() };
        let arr = match save.get("list") { Some(Value::Array(a)) => a.clone(), _ => panic!() };
        assert_eq!(arr[1].to_display(), "X");
    }
    
    #[test]
    fn test_set_path_triple_nest() {
        let mut env = make_env();
        env.set_path(&[PathSegment::Key("save".to_string()), PathSegment::Key("a".to_string()), PathSegment::Key("b".to_string()), PathSegment::Key("c".to_string())], &AssignOp::Set, Value::Number(999.0));
        let path = vec![PathSegment::Key("save".to_string()), PathSegment::Key("a".to_string()), PathSegment::Key("b".to_string()), PathSegment::Key("c".to_string())];
        assert_eq!(env.get_path(&path).as_number(), 999.0);
    }


    #[test]
    fn test_set_var_path_updates_local_scope() {
        let mut env = make_env();
        env.push_scope();
        env.set_local("m", Value::Map({
            let mut m = IndexMap::new();
            m.insert("a".to_string(), Value::Number(1.0));
            m
        }));
        env.set_var_path(&[PathSegment::Key("m".to_string()), PathSegment::Key("a".to_string())], &AssignOp::Set, Value::Number(2.0));
        match env.get_path(&[PathSegment::Key("m".to_string()), PathSegment::Key("a".to_string())]) {
            Value::Number(n) => assert_eq!(n, 2.0),
            other => panic!("ローカルmapのaが更新されているべき: {:?}", other),
        }
    }

    #[test]
    fn test_set_var_path_falls_back_to_globals_when_no_local() {
        let mut env = make_env();
        env.set_var_path(&[PathSegment::Key("save".to_string()), PathSegment::Key("count".to_string())], &AssignOp::Set, Value::Number(5.0));
        let save = match env.globals.get("save") { Some(Value::Map(m)) => m.clone(), _ => panic!() };
        assert_eq!(save["count"].as_number(), 5.0);
    }

}  // ← set_path_testsの閉じ括弧はこの後


