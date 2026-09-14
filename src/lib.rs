// lib.rs
// SSPとのDLL連携
#[macro_use]
mod log;
mod sstp; 
mod parser;
mod codegen;
mod config;
mod runtime;
mod saori;
mod analyzer;

use std::collections::{HashMap, HashSet};
use std::ffi::c_long;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Instant, Duration};
use once_cell::sync::Lazy;
use indexmap::IndexMap;
use codegen::Codegen;

use crate::codegen::Value;
use config::Config;
use parser::{load_program, Talk, LoadError,Stmt};

use winapi::um::winbase::{GlobalAlloc, GlobalFree, GMEM_FIXED};
use winapi::shared::minwindef::HGLOBAL;
use crate::analyzer::Analyzer;

use std::sync::atomic::{AtomicBool, Ordering};


static DEBUG_LOG: AtomicBool = AtomicBool::new(false);

/// このロード期間中に一度でもパニックが起きたか。
/// requestはcatch_unwindでパニックを握り潰すため、パニック後も
/// 通常どおりリクエスト処理が続き、中途半端なcodegen状態のまま
/// unload/loaduでsave.jsonが上書きされうる。保存自体は続けるが
/// （パニック後の正常な変更まで捨てるほうが損失が大きい）、
/// 上書き前の内容を.bakに退避してユーザーが手で戻せるようにする。
static PANICKED: AtomicBool = AtomicBool::new(false);
// ── グローバル状態 ────────────────────────────────────────


struct ManatoState {
    codegen: Codegen,
    talks: HashMap<String, Vec<Talk>>,
    ghost_dir: PathBuf,
    next_talk_time: Instant,
    virtual_time: Option<(i32, u32, u32, u32, u32, u32)>,
    parse_error: Option<String>,
    /// 台本のロードに失敗した状態か。parse_errorはOnBootで一度だけ返すため
    /// takeされて消えるが、こちらは消えない。save.jsonの上書きガードに使う。
    /// この状態のcodegenはglobalsを読み込んでいないので、保存すると
    /// ユーザーのセーブデータが空で潰れる。
    load_failed: bool,
    /// トップレベルglobal評価時に出たエラー。最初のイベント応答で一度だけ返す。
    init_errors: Vec<(String, String)>,
    talk_interval_secs: u64,
    talk_jitter_secs: u64,
    hwnd: u64,
      status_raw: String,
}


static STATE: Lazy<Mutex<Option<ManatoState>>> = Lazy::new(|| Mutex::new(None));

static LOG_DIR: Lazy<Mutex<PathBuf>> = Lazy::new(|| Mutex::new(PathBuf::new()));
use encoding_rs::SHIFT_JIS;

const PERSISTED_SYSTEM_KEYS: &[&str] = &["talk_interval", "talk_jitter", "debug_log"];
const PANIC_BAK: &str = "save.json.panic.bak";
const PANIC_BAK_NOTIFIED: &str = "save.json.panic.bak.notified";
const CORRUPT_BAK: &str = "save.json.corrupt.bak";

/// SSPからFFI境界で渡される長さ(c_long、符号あり)をusizeへ安全に変換する。
/// 負値をそのまま`as usize`すると巨大な値になり、`from_raw_parts`で
/// 範囲外メモリ読み取りにつながる。SSPは通常正しい値を渡すが、
/// 呼び出し元の実装バグ等に対する防御として下限0にクランプする。
fn safe_len(len: c_long) -> usize {
    len.max(0) as usize
}

// ═══════════════════════════════════════════════════════════
// ② append_log 関数の直後に以下を追加
// ═══════════════════════════════════════════════════════════

#[no_mangle]
pub extern "C" fn loadu(h: HGLOBAL, len: c_long) -> i32 {
    append_log!(format!("loadu start"));
    std::panic::catch_unwind(|| {
        if h.is_null() {
        append_log!(format!("loadu: h is null"));
            return 0;
        }
             {
            let mut s = lock_state();
            if s.is_some() {
                // 既存stateがあれば先に保存してから解放
                if let Some(state) = s.as_ref() {
    if !state.load_failed {
        if let Err(_e) = save_globals(&state.codegen, &state.ghost_dir) {
            append_log!(format!("save error: {}", _e));
        }
    }
}
                *s = None;
                append_log!(format!("loadu: previous state cleared"));
            }
        }
        let dir = unsafe {
            let bytes = std::slice::from_raw_parts(h as *const u8, safe_len(len));
            let s = std::str::from_utf8(bytes).unwrap_or("").trim_end_matches('\0').trim_end_matches('\\').trim_end_matches('/');
            let path = PathBuf::from(s);
            GlobalFree(h);
            path
        };
        append_log!(format!("loadu dir: {:?}", dir));

          match init(&dir) {
            Ok(state) => {
                append_log!(format!("init ok, trying lock"));
                let mut s = lock_state();
                *s = Some(state);
                append_log!(format!("state set ok"));
                1
            }
            Err(_e) => {
                append_log!(format!("init error: {}", _e));
                0
            }
        }
    }).unwrap_or_else(|_| {
        append_log!(format!("loadu: panic in catch_unwind"));
        0
    })
}

#[no_mangle]
pub extern "C" fn load(h: HGLOBAL, len: c_long) -> i32 {
    append_log!(format!("load called"));
    std::panic::catch_unwind(|| {
        if h.is_null() {
            append_log!(format!("load: h is null"));
            return 0;
        }
        // ★ loaduで初期化済みなら何もせずhだけ解放して成功扱いにする
        //   （DLL共通仕様: 「loaduで初期化済の時にloadも呼ばれた場合は無視するのが望ましい」）
        if is_already_initialized() {
            unsafe { GlobalFree(h); }
            append_log!(format!("load: already initialized by loadu, ignoring"));
            return 1;
        }
        let dir = unsafe {
            let bytes = std::slice::from_raw_parts(h as *const u8, safe_len(len));
            let (s, _, _) = SHIFT_JIS.decode(bytes);
            let path = PathBuf::from(s.trim_end_matches('\0').trim_end_matches('\\').trim_end_matches('/'));
            GlobalFree(h);
            path
        };
        append_log!(format!("load dir: {:?}", dir));

         match init(&dir) {
            Ok(state) => {
                append_log!(format!("load: init ok, trying lock"));
                let mut s = lock_state();
                *s = Some(state);
                append_log!(format!("load: state set ok"));
                1
            }
            Err(_e) => {
                append_log!(format!("load: init error: {}", _e));
                0
            }
        }
    }).unwrap_or_else(|_| {
        append_log!(format!("load: panic in catch_unwind"));
        0
    })
}

// unload() から drain ブロックを削除
#[no_mangle]
pub extern "C" fn unload() -> i32 {
    std::panic::catch_unwind(|| {
        {
            let mut s = lock_state();
               if let Some(state) = s.as_ref() {
    if !state.load_failed {
        if let Err(_e) = save_globals(&state.codegen, &state.ghost_dir) {
            append_log!(format!("save error: {}", _e));
        }
    }
}
            *s = None;
        }
        1
    }).unwrap_or(1)
}
#[no_mangle]
pub extern "C" fn request(h: HGLOBAL, len: *mut c_long) -> HGLOBAL {
    append_log!(format!("request called"));
    if h.is_null() {
        append_log!(format!("request: h is null"));
        return std::ptr::null_mut();
    }
    let response = std::panic::catch_unwind(|| {
        unsafe {
            let bytes = std::slice::from_raw_parts(h as *const u8, safe_len(*len));
            let req_str = SHIFT_JIS.decode(bytes).0.into_owned(); // ← Cowを所有権ありStringに変換

            let result = handle_request(&req_str); // ← GlobalFreeより先に処理を終わらせる

            GlobalFree(h); // ← 解放はこの位置に移動

            result
        }
    }).unwrap_or_else(|e| {
        let _msg = e.downcast_ref::<&str>().map(|s| *s)
            .or_else(|| e.downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("unknown panic");
        append_log!(format!("request: panic: {}", _msg));
        PANICKED.store(true, Ordering::Relaxed);
        error_response("panic")
    });

    unsafe {
        let (encoded, _, _) = SHIFT_JIS.encode(&response);
        let bytes = encoded.as_ref();
        let size = bytes.len() + 1;
        let mem = GlobalAlloc(GMEM_FIXED, size);
        if mem.is_null() {
            // OOM等でメモリ確保に失敗。nullへ書き込むとクラッシュするため、
            // ここで諦めてnullを返す（SSP側は失敗として扱う）。
            append_log!(format!("request: GlobalAlloc failed"));
            *len = 0;
            return std::ptr::null_mut();
        }
        let ptr = mem as *mut u8;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        *ptr.add(bytes.len()) = 0;
        *len = bytes.len() as c_long;
        mem
    }
}

fn load_program_guarded(
    main: &Path,
) -> Result<
    (Vec<Talk>, Vec<(String, Vec<String>, Vec<Stmt>)>, Vec<(Vec<parser::PathSegment>, parser::AssignOp, parser::Expr)>),
    LoadError
> {
    let main = main.to_path_buf();

    let spawned = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024) // 16MB
        .spawn(move || {
            let mut visited = HashSet::new();
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                load_program(&main, &mut visited)
            }))
        });

    let handle = match spawned {
        Ok(h) => h,
        Err(e) => {
            append_log!(format!("loadu: パーススレッドの起動に失敗しました: {}", e));
            return Err(LoadError::PreprocessError(
                "パース処理を開始できませんでした（システムのリソース不足の可能性があります）。少し待ってから再度お試しください。".to_string()
            ));
        }
    };
    let join_result = handle.join();

    match join_result {
        Ok(Ok(parse_result)) => parse_result,
        Ok(Err(_panic)) => {
            append_log!("loadu: parser panicked (caught)");
            Err(LoadError::PreprocessError(
                "パース中に内部エラーが発生しました。トーク定義の「=>」忘れや、構文の記法を確認してください。".to_string()
            ))
        }
        Err(_) => {
            append_log!("loadu: parser thread crashed (stack overflow likely)");
            Err(LoadError::PreprocessError(
                "パース中にスタックオーバーフローが発生した可能性があります。スクリプトの記法（特にトーク定義の「=>」やブロックの閉じ忘れ）を確認してください。".to_string()
            ))
        }
    }
}

// ── 初期化 ───────────────────────────────────────────────
    
    fn init(dir: &Path) -> Result<ManatoState, String> {
    // 新しいロード期間の開始。前回のパニック記録を引き継がない。
    PANICKED.store(false, Ordering::Relaxed);

    // LOG_DIR を ghost_dir に設定
    if let Ok(mut d) = LOG_DIR.lock() {
        *d = dir.to_path_buf();
    }
    let config_path = dir.join("config.toml");
    append_log!(format!("config path: {:?} exists: {}", config_path, config_path.exists()));

      
    let config = Config::load(&config_path)
        .map_err(|e| format!("config.toml: {}", e))?;
    append_log!(format!("config loaded"));

        DEBUG_LOG.store(config.settings.debug_log, Ordering::Relaxed);
    append_log!(format!("debug_log enabled"));
    let mut init_errors: Vec<(String, String)> = Vec::new();
let panic_bak = dir.join(PANIC_BAK);
if panic_bak.exists() && !dir.join(PANIC_BAK_NOTIFIED).exists() {
    init_errors.push((
        "warning".to_string(),
        format!(
            "前回の実行で内部エラーが発生し、save.jsonが不完全な可能性があります。直前の内容は{}に残っています",
            PANIC_BAK
        ),
    ));
    if let Err(_e) = std::fs::write(dir.join(PANIC_BAK_NOTIFIED), b"") {
        append_log!(format!("notified marker write failed: {}", _e));
    }
}
    let main = dir.join("talks").join("main.mnt");
    append_log!(format!("main.mnt: {:?} exists: {}", main, main.exists()));

  
    append_log!(format!("before load_program"));  
// ↓ クローンしてクロージャに渡す
let _main_for_err = main.clone();


// ── lib.rs の init 内、load_program の呼び出し箇所を変更 ─
// 変更後
let (all_talks, all_funcs, all_globals) = match load_program_guarded(&main) {
    Ok(result) => result,

    Err(LoadError::PreprocessError(msg)) => {
        // preprocessエラーはそのままメッセージを使う
        let state = ManatoState {
            codegen: Codegen::new(config.characters.clone(), HashMap::new(),dir.to_path_buf()) ,
            talks: HashMap::new(),
            ghost_dir: dir.to_path_buf(),
            next_talk_time: next_talk_time(config.settings.talk_interval_secs, config.settings.talk_jitter_secs),
            virtual_time: None,
            parse_error: Some(msg),
            load_failed:true,
           init_errors:  init_errors,
            talk_interval_secs: config.settings.talk_interval_secs,
            talk_jitter_secs: config.settings.talk_jitter_secs,
             hwnd: 0,
               status_raw: String::new(),
            
        };
        return Ok(state);
    }
Err(LoadError::ParseError(msgs, _err_path)) => {
    let msg = msgs.join("\\n");

    let state = ManatoState {
        codegen: Codegen::new(config.characters.clone(), HashMap::new(), dir.to_path_buf()),
        talks: HashMap::new(),
        ghost_dir: dir.to_path_buf(),
        next_talk_time: next_talk_time(config.settings.talk_interval_secs, config.settings.talk_jitter_secs),
        virtual_time: None,
        parse_error: Some(msg),
        load_failed:true,
        init_errors: init_errors,
        talk_interval_secs: config.settings.talk_interval_secs,
        talk_jitter_secs: config.settings.talk_jitter_secs,
        hwnd: 0,
         status_raw: String::new(),
       
    };
    return Ok(state);
}
};

append_log!(format!("all_talks events: {:?}", all_talks.iter().map(|t| &t.event).collect::<Vec<_>>()));

// func_namesを先に作る（all_funcsをムーブする前に）
let func_names: HashSet<String> = all_funcs.iter()
    .map(|(name, _, _)| name.clone())
    .collect();

// talksのHashMapを作る
let mut talks: HashMap<String, Vec<Talk>> = HashMap::new();
for talk in all_talks {
    talks.entry(talk.event.clone()).or_default().push(talk);
}

// talk_namesを作る
let talk_names: HashSet<String> = talks.keys().cloned().collect();

// 静的チェック（all_funcsをムーブする前に）
let analyze_errors = Analyzer::new(talk_names, func_names)
    .analyze(&talks, &all_funcs);

let error_only: Vec<_> = analyze_errors.iter()
    .filter(|e| e.level == "error")
    .collect();

if !error_only.is_empty() {
    let msg = error_only.iter()
        .map(|e| format!("{}内: {}", e.event, e.message))
        .collect::<Vec<_>>()
        .join("\\n");
    // ... return Ok(state) でブロック

    let state = ManatoState {
        codegen: Codegen::new(config.characters.clone(), HashMap::new(), dir.to_path_buf()),
        talks: HashMap::new(),
        ghost_dir: dir.to_path_buf(),
        next_talk_time: next_talk_time(config.settings.talk_interval_secs, config.settings.talk_jitter_secs),
        virtual_time: None,
        parse_error: Some(msg),
        load_failed:true,
        init_errors: init_errors,
        talk_interval_secs: config.settings.talk_interval_secs,
        talk_jitter_secs: config.settings.talk_jitter_secs,
        hwnd: 0,
        status_raw: String::new(),
       
    };
    return Ok(state);
}
           for _e in analyze_errors.iter().filter(|e| e.level == "notice") {
    append_log!(format!("notice: {}内: {}", _e.event, _e.message));
}

// ★追加: 静的チェックのwarningは最初のイベント応答で一度だけ返す
let analyze_warnings: Vec<(String, String)> = analyze_errors.iter()
    .filter(|e| e.level == "warning")
    .map(|e| ("warning".to_string(), format!("{}内: {}", e.event, e.message)))
    .collect();

if let Some(_v) = talks.get("OnMouseDoubleClick") {
append_log!(format!("OnMouseDoubleClick body : {:?}", _v));
}

let characters = config.characters.clone();
let mut codegen = Codegen::new(characters, talks.clone(), dir.to_path_buf());
codegen.auto_newline = config.settings.auto_newline;
// トップレベル関数を登録
for (name, params, body) in all_funcs {
    codegen.env.funcs.insert(name, (params, body));
}

// ① system.* のデフォルトをconfig.tomlの値でまず入れる
//    （この後の load_globals / apply_top_globals は上書きではなくマージする）
let mut system_map: IndexMap<String, Value> = IndexMap::new();
system_map.insert("talk_interval".to_string(), Value::Number(config.settings.talk_interval_secs as f64));
system_map.insert("talk_jitter".to_string(),   Value::Number(config.settings.talk_jitter_secs as f64));
system_map.insert("debug_log".to_string(),     Value::Bool(config.settings.debug_log));
system_map.insert("ghost_dir".to_string(),     Value::Str(dir.to_string_lossy().to_string()));
system_map.insert("version".to_string(),       Value::Str(env!("CARGO_PKG_VERSION").to_string()));
codegen.env.globals.insert("system".to_string(), Value::Map(system_map));
append_log!(format!("system.* defaults initialized"));


// ② save.jsonの永続値をマージ（system.*はフィールド単位マージ、他はそのまま）
let persisted_system = load_globals(&mut codegen, dir)?;
append_log!(format!("globals loaded"));

// ③ 台本トップレベルの global 文を適用（save.*の?=等の初期化パターンのため、
//    load_globalsより後に実行する必要がある）
apply_top_globals(&mut codegen, &all_globals);
append_log!(format!("top-level globals applied"));

// system.*だけは、台本が何を書いていてもsave.jsonの永続値を最終的に勝たせる。
// 台本トップレベルの `global system.talk_interval = ...` が、
// ユーザーが実行中に変更してsave.jsonへ書いた値を毎回上書きしてしまう問題への対処。
if let Some(sys) = persisted_system {
    merge_system_globals(&mut codegen.env.globals, sys);
}

// トップレベルglobalの評価中に出たエラーをここで回収する。
// 放置すると最初のイベント（通常OnBoot）のerrorsに混ざり、
// OnBootのトークが原因であるかのように表示されてしまう。
let top_global_errors: Vec<(String, String)> = codegen.errors.drain(..)
  .filter(|(level, _)| level == "error" || level == "warning")
    .map(|(level, msg)| (level, format!("台本トップレベルのglobal文: {}", msg)))
    .collect();
for (_level, _msg) in &top_global_errors {
    append_log!(format!("top-level global error: {}: {}", _level, _msg));
}
   init_errors.extend(analyze_warnings);
init_errors.extend(top_global_errors);
// ④ 最終的なsystem.*の値をglobalsから読み取り、起動時のtalk_interval/jitter/debug_logに反映
let mut talk_interval_secs = config.settings.talk_interval_secs;
let mut talk_jitter_secs = config.settings.talk_jitter_secs;
if let Some(codegen::Value::Map(ref m)) = codegen.env.globals.get("system") {
    if let Some(v) = m.get("talk_interval") {
        talk_interval_secs = v.as_number() as u64;
        append_log!(format!("talk_interval overridden: {}", talk_interval_secs));
    }
    if let Some(v) = m.get("talk_jitter") {
        talk_jitter_secs = v.as_number() as u64;
        append_log!(format!("talk_jitter overridden: {}", talk_jitter_secs));
    }
    if let Some(v) = m.get("debug_log") {
        DEBUG_LOG.store(v.as_bool(), Ordering::Relaxed);
        append_log!(format!("debug_log overridden: {}", v.as_bool()));
    }
}

Ok(ManatoState {
    codegen,
    talks,
    ghost_dir: dir.to_path_buf(),
    next_talk_time: next_talk_time(talk_interval_secs, talk_jitter_secs),
    virtual_time: None,
    parse_error: None,
    load_failed:false,
    init_errors,
    talk_interval_secs,
    talk_jitter_secs,
    hwnd: 0,
     status_raw: String::new(),
})
}

fn apply_top_globals(
    codegen: &mut Codegen,
    globals: &[(Vec<parser::PathSegment>, parser::AssignOp, parser::Expr)],
) {
    for (path, op, expr) in globals {
        let val = codegen.eval_expr_full(expr);
        let resolved = codegen.resolve_path_segments(path);
        codegen.env.set_path_resolved(&resolved, op, val);
        append_log!(format!("top-level global applied: {:?} {:?}", resolved, op));
    }
}

// ── リクエスト処理 ────────────────────────────────────────

fn handle_request(req: &str) -> String {
    let refs = parse_all_references(req);
    let event = parse_event(req);
    append_log!(format!("handle_request: raw event=[{}], req.len={}", event, req.len()));
 
if let Some(hwnd_val) = parse_header(req, "HWnd") {
    let mut s = lock_state();
    if let Some(state) = s.as_mut() {
        if let Ok(h) = hwnd_val.parse::<u64>() {
            state.hwnd = h;
        }
    }
}
// Statusヘッダは「無い＝その状態ではない」を意味するため、
// ヘッダが無いリクエストでも必ず上書きする。
// if let Some(..) にすると前回の値が残り続け、
// 発話が終わったあとも status.talking が true のままになる。
{
    let status_val = parse_header(req, "Status").unwrap_or_default();
    let mut s = lock_state();
    if let Some(state) = s.as_mut() {
        state.status_raw = status_val;
    }
}

    // ★変更: On始まりでないID（Resourceリクエスト）のうち、
    //   version/craftmanだけは値を返す。それ以外は従来通り204。
    if !event.starts_with("On") {
        return match event.as_str() {
            "version" => shiori_resource_response(env!("CARGO_PKG_VERSION")),
            "craftman" => 
                shiori_resource_response("mizuki"),
            
            _ => not_found_response(),
        };
    }

    // ...以降は変更なし
    

    let mut s = lock_state();
    let state = match s.as_mut() {
        Some(s) => s,
        None => return error_response("not initialized"),
    };
if state.parse_error.is_some() {
    if event == "OnBoot" {
        let msg = state.parse_error.take().unwrap();
        let mut errors = std::mem::take(&mut state.init_errors);
        errors.push(("error".to_string(), msg.clone()));
        return shiori_response_with_error(
            &format!("\\b[2]\\0パースエラー:\\n{}\\e", msg),
            &errors
        );
    }
    return not_found_response();
}
    
    // ランダムトーク判定
let actual_event = if event == "OnMinuteChange" {
    // ↓先に仮想時刻取得を返す
    let script = format!(
        "\\![get,property,OnGotVirtualTime,system.year,system.month,system.day,system.hour,system.minute,system.second]\\e"
    );
    if parse_reference(req, 3) != "1" {
        return shiori_response(&script);
    }
    if Instant::now() < state.next_talk_time {
        return shiori_response(&script);
    }
    state.next_talk_time = next_talk_time(state.talk_interval_secs, state.talk_jitter_secs);
    "OnRandomTalk".to_string()
    } else if event == "OnAITalk" {
        // \a やメニューからの手動ランダムトーク
        "OnRandomTalk".to_string()
    } else {
        event.clone()
    };
if actual_event == "OnGotVirtualTime" {
    let y  = parse_reference(req, 0).parse::<i32>().unwrap_or(2026);
    let mo = parse_reference(req, 1).parse::<u32>().unwrap_or(1);
    let d  = parse_reference(req, 2).parse::<u32>().unwrap_or(1);
    let h  = parse_reference(req, 3).parse::<u32>().unwrap_or(0);
    let mi = parse_reference(req, 4).parse::<u32>().unwrap_or(0);
    let s  = parse_reference(req, 5).parse::<u32>().unwrap_or(0);
    state.virtual_time = Some((y, mo, d, h, mi, s));
    append_log!(format!("virtual_time set: {}-{}-{} {}:{}:{}", y, mo, d, h, mi, s));
    return not_found_response();
}


// 変更後
let ghost_dir = state.ghost_dir.clone();
if DEBUG_LOG.load(Ordering::Relaxed) {
    let _ = std::fs::write(ghost_dir.join("minato_request.log"), req);
}
   let candidates = match state.talks.get(&actual_event) {
    Some(c) if !c.is_empty() => c.clone(),
    _ => return not_found_response(),
};

// Statusヘッダをtalk側から参照できる形に展開する
let mut status_map = IndexMap::new();
for flag in ["talking", "choosing", "minimizing", "induction", "passive", "timecritical", "nouserbreak", "online"] {
  
status_map.insert(
    flag.to_string(),
    Value::Bool(state.status_raw.split(',').any(|s| s.trim() == flag)),
);
}
status_map.insert("raw".to_string(), Value::Str(state.status_raw.clone()));
state.codegen.env.globals.insert("status".to_string(), Value::Map(status_map));

    let script = match state.codegen.gen_event(
        &actual_event,
        &candidates,
        &refs,
        state.virtual_time,
    ) {
        Some(s) => s,
    None => {
    let mut errors: Vec<(String, String)> = state.codegen.errors.drain(..).collect();
    if DEBUG_LOG.load(Ordering::Relaxed) {
        let mut all = std::mem::take(&mut state.init_errors); all.append(&mut errors);
        if !all.is_empty() {
            append_log!(format!("空出力をデバッグ表示: event={}, errors={:?}", actual_event, all));
            return shiori_response_with_error("\\e", &all);
        }
    }
    return not_found_response();
}
    };

    // ↓ 追加
// OnRandomTalk / OnAITalk の結果を save.last_talk に自動保存
if actual_event == "OnRandomTalk" || actual_event == "OnAITalk" {
    let text = script.trim_end_matches("\\e").to_string();
    state.codegen.env.set_path_resolved(
        &["save".to_string(), "last_talk".to_string()],
        &parser::AssignOp::Set,
        codegen::Value::Str(text),
    );
}
let mut errors: Vec<(String, String)> = std::mem::take(&mut state.init_errors);
errors.extend(state.codegen.errors.drain(..));

// ↓ 追加: system.* の変更を STATE に即時反映
if let Some(Value::Map(ref m)) = state.codegen.env.globals.get("system") {
    if let Some(v) = m.get("talk_interval") {
        state.talk_interval_secs = v.as_number() as u64;
    }
    if let Some(v) = m.get("talk_jitter") {
        state.talk_jitter_secs = v.as_number() as u64;
    }
    if let Some(v) = m.get("debug_log") {
        DEBUG_LOG.store(v.as_bool(), Ordering::Relaxed);
    }
}

// OnBootとOnMinuteChangeは末尾にget,propertyを付け足す
if actual_event == "OnBoot" || actual_event == "OnMinuteChange" {
    let script_without_e = script.trim_end_matches("\\e");
    let full_script = format!(
        "{}\\![get,property,OnGotVirtualTime,system.year,system.month,system.day,system.hour,system.minute,system.second]\\e",
        script_without_e
    );
    return if errors.is_empty() {
        shiori_response(&full_script)
    } else {
        shiori_response_with_error(&full_script, &errors)
    };
}
    if errors.is_empty() {
        shiori_response(&script)
    } else {
        shiori_response_with_error(&script, &errors)
    }
}

// ── SHIORI/3.0 プロトコル ─────────────────────────────────

fn parse_event(req: &str) -> String {
    for line in req.lines() {
        if line.starts_with("ID: ") {
            return line["ID: ".len()..].trim().to_string();
        }
    }
    String::new()
}

fn parse_reference(req: &str, n: usize) -> String {
    let key = format!("Reference{}", n);
    for line in req.lines() {
        if line.starts_with(&format!("{}: ", key)) {
            return line[key.len() + 2..].trim().to_string();
        }
    }
    String::new()
}
fn parse_all_references(req: &str) -> HashMap<String, String> {
    let mut refs: HashMap<String, String> = HashMap::new();
    for line in req.lines() {
        if let Some(rest) = line.strip_prefix("Reference") {
            append_log!(format!("ref_line=[{}]", line));
            if let Some((n, val)) = rest.split_once(": ") {
                refs.insert(n.to_string(), val.trim().to_string());
            }
        }
    }
    refs
}

use std::sync::atomic::AtomicU32;

static RNG_STATE: AtomicU32 = AtomicU32::new(0);

/// SplitMix32。
/// 以前はxorshift32を使っていたが、下位ビットの質が悪く、
/// 小さな法での剰余演算で強い周期性・偏りが出ていたため置き換えた。
pub fn next_rand() -> u32 {
    let mut x = RNG_STATE.load(Ordering::Relaxed);
    if x == 0 {
        use std::time::{SystemTime, UNIX_EPOCH};
        x = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32))
            .unwrap_or(12345);
        if x == 0 { x = 12345; }
    }
    x = x.wrapping_add(0x9E3779B9);
    RNG_STATE.store(x, Ordering::Relaxed);

    let mut z = x;
    z = (z ^ (z >> 16)).wrapping_mul(0x21F0AAAD);
    z = (z ^ (z >> 15)).wrapping_mul(0x735A2D97);
    z ^ (z >> 15)
}
/// 次回トーク予定時刻を計算（基本間隔＋揺らぎ）
///
/// interval/jitterはsave.jsonの改ざん・破損や台本の`global system.talk_jitter = ...`
/// 経由で任意のu64値になりうる。巨大な値（例: u64::MAX）だと
/// `jitter + 1`が0にラップして`%`が0除算パニックになったり、
/// `Instant::now() + Duration::from_secs(...)`がInstantの表現可能範囲を
/// 超えてパニックしうるため、現実的な上限でクランプしてから計算する。
const MAX_TALK_DELAY_SECS: u64 = 60 * 60 * 24 * 30; // 30日

fn next_talk_time(interval: u64, jitter: u64) -> Instant {
    let interval = interval.min(MAX_TALK_DELAY_SECS);
    let jitter = jitter.min(MAX_TALK_DELAY_SECS);
    let j = if jitter > 0 { next_rand() as u64 % (jitter + 1) } else { 0 };
    Instant::now() + Duration::from_secs(interval + j)
}

fn shiori_response(script: &str) -> String {
        append_log!(format!("★RESPONSE 200★ value=[{}]", script));
    format!(
        "SHIORI/3.0 200 OK\r\nCharset: Shift_JIS\r\nSender: minato\r\nValue: {}\r\n\r\n",
        script
    )
}
// lib.rs — shiori_response の近くに追加

/// version/craftman等のResourceリクエストへの応答。
/// SHIORI/3.0上はshiori_responseと同じ「200 OK + Value:」形式だが、
/// 台本のトーク出力ではなく固定的なメタ情報を返す用途として区別する。
fn shiori_resource_response(value: &str) -> String {
    format!(
        "SHIORI/3.0 200 OK\r\nCharset: Shift_JIS\r\nSender: minato\r\nValue: {}\r\n\r\n",
        value
    )
}
fn shiori_response_with_error(script: &str, errors: &[(String, String)]) -> String {
    let levels: String = errors.iter()
        .map(|(l, _)| l.as_str())
        .collect::<Vec<_>>()
        .join("\x01");
    let descs: String = errors.iter()
        .map(|(_, d)| d.as_str())
        .collect::<Vec<_>>()
        .join("\x01");
    format!(
        "SHIORI/3.0 200 OK\r\nCharset: Shift_JIS\r\nSender: minato\r\nValue: {}\r\nErrorLevel: {}\r\nErrorDescription: {}\r\n\r\n",
        script, levels, descs
    )
}

fn not_found_response() -> String {
       append_log!(format!("★NOT_FOUND★ called"));
    "SHIORI/3.0 204 No Content\r\nCharset: Shift_JIS\r\nSender: minato\r\n\r\n".to_string()
}

fn error_response(msg: &str) -> String {
    format!(
        "SHIORI/3.0 400 Bad Request\r\nCharset: Shift_JIS\r\nSender: minato\r\nValue: [minato error: {}]\r\n\r\n",
        msg
    )
}

// ── セーブデータ ──────────────────────────────────────────

fn save_globals(codegen: &Codegen, dir: &Path) -> Result<(), String> {
    use std::io::Write;
    
    let mut json_map: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    
    // save.* を保存
    if let Some(Value::Map(m)) = codegen.env.globals.get("save") {
        let save_json: serde_json::Map<String, serde_json::Value> = m.iter()
            .map(|(k, v)| (k.clone(), value_to_json(v)))
            .collect();
        json_map.insert("save".to_string(), serde_json::Value::Object(save_json));
    }
    
    // system.* を保存（読み取り専用項目は除く）
    if let Some(Value::Map(m)) = codegen.env.globals.get("system") {
        let mut system_json: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
        for key in  PERSISTED_SYSTEM_KEYS {
            if let Some(v) = m.get(*key) {
                system_json.insert(key.to_string(), value_to_json(v));
            }
        }
        if !system_json.is_empty() {
            json_map.insert("system".to_string(), serde_json::Value::Object(system_json));
        }
    }
    
    if json_map.is_empty() { return Ok(()); }
        
    let json = serde_json::to_string_pretty(&json_map).map_err(|e| e.to_string())?;
    let path = dir.join("save.json");

    if PANICKED.load(Ordering::Relaxed) && path.exists() {
    let bak = dir.join(PANIC_BAK);
    match std::fs::copy(&path, &bak) {
        Ok(_) => {
            // 新しいインシデントなので、前回の通知済みマーカーは無効化する
            let _ = std::fs::remove_file(dir.join(PANIC_BAK_NOTIFIED));
            append_log!("panic backup written to save.json.panic.bak");
        }
        Err(_e) => append_log!(format!("panic backup failed: {}", _e)),
    }
}
    let tmp = dir.join("save.json.tmp");
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// save.jsonのsystem.*を返り値として返す。
/// system.*だけは「台本のトップレベルglobalより、save.jsonに永続化された値を
/// 常に優先する」ため、apply_top_globals適用後にもう一度このsystem.*を
/// 上書きマージする必要があり、その材料として呼び出し元に持ち帰らせる。
fn load_globals(codegen: &mut Codegen, dir: &Path) -> Result<Option<Value>, String> {
    let path = dir.join("save.json");
    if !path.exists() { return Ok(None); }
    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let val = match serde_json::from_str(&json) {
        Ok(v) => v,
        Err(e) => {
            let _msg = format!("save.json corrupt, resetting: {}", e);
            append_log!(_msg);
      let _ = std::fs::rename(&path, dir.join(CORRUPT_BAK));
            return Ok(None);
        }
    };
    let mut persisted_system: Option<Value> = None;
    if let serde_json::Value::Object(map) = val {
        for (k, v) in map {
            let value = json_to_value(v);
            if k == "system" {
                merge_system_globals(&mut codegen.env.globals, value.clone());
                persisted_system = Some(value);
            } else {
                codegen.env.globals.insert(k, value);
            }
        }
    }
    Ok(persisted_system)
}
/// save.json由来のsystem.*を、既存のsystem Map（config.tomlデフォルト＋ghost_dir/version）に
/// フィールド単位で上書きマージする。loaded側に存在しないキー（ghost_dir/versionなど）は保持される。
fn merge_system_globals(globals: &mut HashMap<String, Value>, loaded: Value) {
    let loaded_map = match loaded {
        Value::Map(m) => m,
        _ => return,
    };
    let existing = globals.remove("system").unwrap_or(Value::Map(IndexMap::new()));
    let mut existing_map = match existing {
        Value::Map(m) => m,
        _ => IndexMap::new(),
    };
for (k, v) in loaded_map {
    if PERSISTED_SYSTEM_KEYS.contains(&k.as_str()) {
        existing_map.insert(k, v);
    }
}
    globals.insert("system".to_string(), Value::Map(existing_map));
}
fn json_to_value(v: serde_json::Value) -> codegen::Value {
    match v {
        serde_json::Value::Null => codegen::Value::Null,
        serde_json::Value::Bool(b) => codegen::Value::Bool(b),
        serde_json::Value::Number(n) => codegen::Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => codegen::Value::Str(s),
        serde_json::Value::Array(a) => codegen::Value::Array(a.into_iter().map(json_to_value).collect()),
         serde_json::Value::Object(o) => codegen::Value::Map(
            o.into_iter()
                .map(|(k, v)| (k, json_to_value(v)))
                .collect::<IndexMap<_, _>>()  // ← 変更
        ),
    }
}
fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::Number(n) => serde_json::json!(*n),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Array(a) => serde_json::Value::Array(a.iter().map(value_to_json).collect()),
        Value::Map(m) => serde_json::Value::Object(m.iter().map(|(k,v)| (k.clone(), value_to_json(v))).collect()),
        Value::Null => serde_json::Value::Null,
    }
}


// parse_header ヘルパーを追加（lib.rs 末尾あたりに）
fn parse_header(req: &str, name: &str) -> Option<String> {
    let prefix = format!("{}: ", name);
    for line in req.lines() {
        if line.starts_with(&prefix) {
            return Some(line[prefix.len()..].trim().to_string());
        }
    }
    None
}

/// STATEのロックを取得する。
/// gen_event中のパニックでMutexがpoisonされても、Rust側のデータ構造自体は
/// 有効なままなので、poisonを無視して中身を取り出す。
/// （poisonのまま放置すると、一度のパニックで以降すべてのリクエストが
///   lock errorになり、unloadでのセーブも失われる）
fn lock_state() -> std::sync::MutexGuard<'static, Option<ManatoState>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// STATEが既に初期化済みかどうかを判定する。
/// load()のガード（loaduで初期化済みならloadは無視する）の判定部分を
/// 切り出したもの。HGLOBALを介さずに直接テストできるようにするため。
fn is_already_initialized() -> bool {
    lock_state().is_some()
}
#[cfg(test)]
pub(crate) static TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
// lib.rs の末尾に追加

#[cfg(test)]
mod save_load_tests {
    use super::*;  // lib.rs のスコープなので save_globals/load_globals が見える
    use tempfile::TempDir;
    use indexmap::IndexMap;
    use crate::codegen::{Codegen, Value};

    fn make_gen() -> Codegen {
        let mut chars = HashMap::new();
        chars.insert("湊".to_string(), "\\0".to_string());
        Codegen::new(chars, HashMap::new(), std::path::PathBuf::from("."))
    }

    fn temp_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir作成失敗")
    }

    // ── 基本型のラウンドトリップ ────────────────────────────

    #[test]
    fn test_roundtrip_primitives() {
        let dir = temp_dir();
        let mut cg = make_gen();

        // 数値・文字列・bool・Nullを混在させて保存
        let mut m = IndexMap::new();
        m.insert("訪問回数".to_string(), Value::Number(42.0));
        m.insert("名前".to_string(),     Value::Str("朝霧湊".to_string()));
        m.insert("フラグ".to_string(),   Value::Bool(true));
        m.insert("未設定".to_string(),   Value::Null);
        cg.env.globals.insert("save".to_string(), Value::Map(m));

        // 保存
        save_globals(&cg, dir.path()).expect("save失敗");

        // 別のCodegenで読み直す
        let mut gen2 = make_gen();
        load_globals(&mut gen2, dir.path()).expect("load失敗");

        let save = match gen2.env.globals.get("save") {
            Some(Value::Map(m)) => m.clone(),
            _ => panic!("saveキーが存在しない"),
        };

        assert_eq!(save["訪問回数"].as_number(), 42.0);
        assert_eq!(save["名前"].to_display(),    "朝霧湊");
        assert!(matches!(save["フラグ"], Value::Bool(true)));
        // Nullはsave_globalsでjson::Nullになり復元される
        assert!(matches!(save["未設定"], Value::Null));
    }

    // ── 浮動小数点の精度 ────────────────────────────────────

    #[test]
    fn test_roundtrip_float() {
        let dir = temp_dir();
        let mut cg = make_gen();

        let mut m = IndexMap::new();
        m.insert("体重".to_string(), Value::Number(57.8));
        m.insert("pi".to_string(),   Value::Number(std::f64::consts::PI));
        cg.env.globals.insert("save".to_string(), Value::Map(m));

        save_globals(&cg, dir.path()).expect("save失敗");

        let mut gen2 = make_gen();
        load_globals(&mut gen2, dir.path()).expect("load失敗");

        let save = match gen2.env.globals.get("save") {
            Some(Value::Map(m)) => m.clone(),
            _ => panic!("saveキーが存在しない"),
        };

        // f64 → JSON → f64 で誤差が出ないか
        assert!((save["体重"].as_number() - 57.8).abs() < 1e-10);
        assert!((save["pi"].as_number() - std::f64::consts::PI).abs() < 1e-10);
    }

    // ── ネストしたMap ────────────────────────────────────────

    #[test]
    fn test_roundtrip_nested_map() {
        let dir = temp_dir();
        let mut cg = make_gen();

        // save.stats.win = 10, save.stats.lose = 3
        let mut stats = IndexMap::new();
        stats.insert("win".to_string(),  Value::Number(10.0));
        stats.insert("lose".to_string(), Value::Number(3.0));

        let mut m = IndexMap::new();
        m.insert("stats".to_string(), Value::Map(stats));
        cg.env.globals.insert("save".to_string(), Value::Map(m));

        save_globals(&cg, dir.path()).expect("save失敗");

        let mut gen2 = make_gen();
        load_globals(&mut gen2, dir.path()).expect("load失敗");

        let save = match gen2.env.globals.get("save") {
            Some(Value::Map(m)) => m.clone(),
            _ => panic!("saveキーが存在しない"),
        };

        let stats = match save.get("stats") {
            Some(Value::Map(m)) => m.clone(),
            _ => panic!("stats キーが存在しない"),
        };
        assert_eq!(stats["win"].as_number(),  10.0);
        assert_eq!(stats["lose"].as_number(), 3.0);
    }

    // ── 配列 ─────────────────────────────────────────────────

    #[test]
    fn test_roundtrip_array() {
        let dir = temp_dir();
        let mut cg = make_gen();

        let arr = Value::Array(vec![
            Value::Str("りんご".to_string()),
            Value::Str("みかん".to_string()),
            Value::Number(3.0),
        ]);
        let mut m = IndexMap::new();
        m.insert("履歴".to_string(), arr);
        cg.env.globals.insert("save".to_string(), Value::Map(m));

        save_globals(&cg, dir.path()).expect("save失敗");

        let mut gen2 = make_gen();
        load_globals(&mut gen2, dir.path()).expect("load失敗");

        let save = match gen2.env.globals.get("save") {
            Some(Value::Map(m)) => m.clone(),
            _ => panic!("saveキーが存在しない"),
        };

        let arr = match save.get("履歴") {
            Some(Value::Array(a)) => a.clone(),
            _ => panic!("履歴キーが存在しない"),
        };
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].to_display(), "りんご");
        assert_eq!(arr[1].to_display(), "みかん");
        assert_eq!(arr[2].as_number(),  3.0);
    }

    // ── IndexMapのキー順序 ───────────────────────────────────
    // JSON Object → IndexMap の変換でinsert順が保たれるか
    // serde_json は Object のキー順を保証しないが、
    // save_globals が pretty_print するので順序は記録される。
    // ただし json_to_value 経由で IndexMap に戻す際の順序を確認する。

    #[test]
    fn test_roundtrip_key_order() {
        let dir = temp_dir();
        let mut cg = make_gen();

        let keys = vec!["z", "a", "m", "b"];
        let mut m = IndexMap::new();
        for k in &keys {
            m.insert(k.to_string(), Value::Number(1.0));
        }
        cg.env.globals.insert("save".to_string(), Value::Map(m));

        save_globals(&cg, dir.path()).expect("save失敗");

        let mut gen2 = make_gen();
        load_globals(&mut gen2, dir.path()).expect("load失敗");

        let save = match gen2.env.globals.get("save") {
            Some(Value::Map(m)) => m.clone(),
            _ => panic!("saveキーが存在しない"),
        };

        let restored_keys: Vec<&str> = save.keys().map(|s| s.as_str()).collect();
        // serde_json::Map はキー順を保証しないので、
        // 順序ではなく「全キーが揃っているか」だけ確認する
        for k in &keys {
            assert!(save.contains_key(*k), "キー「{}」が消えた", k);
        }
        assert_eq!(restored_keys.len(), keys.len());
    }


    
    // ── save.jsonが存在しない場合 ────────────────────────────

    #[test]
    fn test_load_no_file() {
        let dir = temp_dir();
        let mut cg = make_gen();
        // ファイルがなくてもエラーにならない
        let result = load_globals(&mut cg, dir.path());
        assert!(result.is_ok());
        // saveキーは存在しない
        assert!(cg.env.globals.get("save").is_none());
    }

    // ── save.jsonが壊れている場合 ────────────────────────────

    #[test]
    fn test_load_corrupt_file() {
        let dir = temp_dir();
        // 不正なJSONを書く
        std::fs::write(dir.path().join("save.json"), b"{ broken json }")
            .expect("書き込み失敗");

        let mut cg = make_gen();
        // パニックせずOkを返し、バックアップを作る
        let result = load_globals(&mut cg, dir.path());
        assert!(result.is_ok());

        // save.json.bak が作られているか
        assert!(dir.path().join("save.json.corrupt.bak").exists(), "bakファイルが作られていない");
        // saveキーは空のまま
        assert!(cg.env.globals.get("save").is_none());
    }

    // ── アトミック書き込み（tmpファイルが残らない）──────────

    #[test]
    fn test_save_no_tmp_remains() {
        let dir = temp_dir();
        let mut cg = make_gen();
        let mut m = IndexMap::new();
        m.insert("x".to_string(), Value::Number(1.0));
        cg.env.globals.insert("save".to_string(), Value::Map(m));

        save_globals(&cg, dir.path()).expect("save失敗");

        // .tmp が残っていないこと
        assert!(!dir.path().join("save.json.tmp").exists(), "tmpファイルが残っている");
        // 正規ファイルは存在する
        assert!(dir.path().join("save.json").exists());
    }



#[cfg(test)]
mod load_guard_tests {
    use super::*;
    use tempfile::TempDir;

    fn make_minimal_ghost_dir() -> TempDir {
        let dir = tempfile::tempdir().expect("tempdir作成失敗");
        std::fs::write(
            dir.path().join("config.toml"),
            "[characters]\n\"湊\" = \"\\\\0\"\n",
        ).expect("config.toml書き込み失敗");
        std::fs::create_dir(dir.path().join("talks")).expect("talksディレクトリ作成失敗");
        std::fs::write(
            dir.path().join("talks").join("main.mnt"),
            "OnBoot => {\n    湊: おはよう\n}\n",
        ).expect("main.mnt書き込み失敗");
        dir
    }
     
            #[test]
fn test_is_already_initialized_reflects_state() {
    let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());

    // 前提: STATEが空
    if let Ok(mut s) = STATE.lock() { *s = None; }
    assert!(!is_already_initialized(), "STATEが空なのにtrueを返している");

    let dir = make_minimal_ghost_dir();
    let state = init(dir.path()).expect("init失敗");
    if let Ok(mut s) = STATE.lock() { *s = Some(state); }

    assert!(is_already_initialized(), "STATEに値があるのにfalseを返している");

    if let Ok(mut s) = STATE.lock() { *s = None; }
}

}

#[test]
fn test_panic_bak_and_corrupt_bak_coexist() {
    let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = temp_dir();
    let mut cg = make_gen();
    let mut m = IndexMap::new();
    m.insert("x".to_string(), Value::Number(1.0));
    cg.env.globals.insert("save".to_string(), Value::Map(m));

    // パニック退避を作る
    PANICKED.store(false, Ordering::Relaxed);
    save_globals(&cg, dir.path()).expect("1回目のsave失敗");
    PANICKED.store(true, Ordering::Relaxed);
    save_globals(&cg, dir.path()).expect("2回目のsave失敗");
    assert!(dir.path().join(PANIC_BAK).exists(), "パニック退避が作られていない");

    // その後にsave.jsonが壊れ、破損退避が走る
    std::fs::write(dir.path().join("save.json"), b"{ broken json }").unwrap();
    let mut gen2 = make_gen();
    load_globals(&mut gen2, dir.path()).expect("load失敗");

    assert!(dir.path().join(CORRUPT_BAK).exists(), "破損退避が作られていない");
    assert!(
        dir.path().join(PANIC_BAK).exists(),
        "破損退避がパニック退避を消している"
    );
    let bak = std::fs::read_to_string(dir.path().join(PANIC_BAK)).unwrap();
    assert!(bak.contains("1"), "パニック退避の中身が壊れている: {}", bak);

    PANICKED.store(false, Ordering::Relaxed);
}


#[test]
fn test_save_json_not_clobbered_after_parse_error_boot() {
    let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(mut s) = STATE.lock() { *s = None; }

    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    std::fs::write(
        dir.path().join("save.json"),
        r#"{"save":{"訪問回数":42}}"#,
    ).expect("save.json書き込み失敗");
    std::fs::write(
        dir.path().join("config.toml"),
        "[characters]\n\"湊\" = \"\\\\0\"\n",
    ).expect("config.toml書き込み失敗");
    std::fs::create_dir(dir.path().join("talks")).expect("talks作成失敗");
    std::fs::write(
        dir.path().join("talks").join("main.mnt"),
        "OnBoot {\n    湊: おはよう\n}\n",
    ).expect("main.mnt書き込み失敗");

    let state = init(dir.path()).expect("init失敗");
    assert!(state.load_failed, "ロード失敗フラグが立っていない");
    if let Ok(mut s) = STATE.lock() { *s = Some(state); }

    // OnBootでparse_errorがtakeされる
    let req = "SEND SHIORI/3.0\r\nID: OnBoot\r\nSender: SSP\r\nCharset: UTF-8\r\n\r\n";
    let _ = handle_request(req);

    unload();

    let json = std::fs::read_to_string(dir.path().join("save.json")).expect("save.jsonが消えている");
    assert!(json.contains("42"), "パースエラー起動でsave.jsonが上書きされた: {}", json);

    if let Ok(mut s) = STATE.lock() { *s = None; }
}

#[test]
fn test_next_talk_time_does_not_panic_on_huge_values() {
    // jitter+1がu64::MAXからラップして0除算になる、あるいはInstant加算が
    // オーバーフローするケースを再現し、パニックしないことを確認する
    let _ = next_talk_time(u64::MAX, u64::MAX);
    let _ = next_talk_time(0, u64::MAX);
    let _ = next_talk_time(u64::MAX, 0);
}

#[test]
fn test_save_json_huge_talk_jitter_does_not_crash_init() {
    // save.jsonが改ざん/破損してtalk_jitterに巨大な数値が入っていても
    // ロード直後の次回トーク時刻計算でパニックしないこと
    let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(mut s) = STATE.lock() { *s = None; }

    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    std::fs::write(
        dir.path().join("save.json"),
        r#"{"system":{"talk_jitter":1e20}}"#,
    ).expect("save.json書き込み失敗");
    std::fs::write(
        dir.path().join("config.toml"),
        "[characters]\n\"湊\" = \"\\\\0\"\n",
    ).expect("config.toml書き込み失敗");
    std::fs::create_dir(dir.path().join("talks")).expect("talks作成失敗");
    std::fs::write(
        dir.path().join("talks").join("main.mnt"),
        "OnBoot => {\n    湊: おはよう\n}\n",
    ).expect("main.mnt書き込み失敗");

    let _state = init(dir.path()).expect("init失敗");

    if let Ok(mut s) = STATE.lock() { *s = None; }
}

#[test]
    fn test_panic_flag_triggers_backup_on_save() {
        let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = temp_dir();
        let mut cg = make_gen();
        let mut m = IndexMap::new();
        m.insert("x".to_string(), Value::Number(1.0));
        cg.env.globals.insert("save".to_string(), Value::Map(m));

        PANICKED.store(false, Ordering::Relaxed);
        save_globals(&cg, dir.path()).expect("1回目のsave失敗");
        assert!(!dir.path().join("save.json.panic.bak").exists(), "パニックしていないのにbakが作られている");

        // 値を変えてから、パニック済みの状態で保存
        cg.env.globals.insert("save".to_string(), Value::Map({
            let mut m = IndexMap::new();
            m.insert("x".to_string(), Value::Number(2.0));
            m
        }));
        PANICKED.store(true, Ordering::Relaxed);
        save_globals(&cg, dir.path()).expect("2回目のsave失敗");

        assert!(dir.path().join("save.json.panic.bak").exists(), "パニック後の保存でbakが作られていない");
        let bak = std::fs::read_to_string(dir.path().join("save.json.panic.bak")).unwrap();
        assert!(bak.contains("1"), "bakが上書き前の内容になっていない: {}", bak);
        let cur = std::fs::read_to_string(dir.path().join("save.json")).unwrap();
        assert!(cur.contains("2"), "本体が最新の内容になっていない: {}", cur);

        PANICKED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn test_backup_skipped_when_no_existing_save() {
        let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = temp_dir();
        let mut cg = make_gen();
        let mut m = IndexMap::new();
        m.insert("x".to_string(), Value::Number(1.0));
        cg.env.globals.insert("save".to_string(), Value::Map(m));

        PANICKED.store(true, Ordering::Relaxed);
        save_globals(&cg, dir.path()).expect("save失敗");
        assert!(!dir.path().join("save.json.panic.bak").exists(), "退避元が無いのにbakが作られている");

        PANICKED.store(false, Ordering::Relaxed);
    }

}

// ★ここから新しい独立モジュール
#[cfg(test)]
mod request_log_tests {
    use super::*;
    use tempfile::TempDir;

    fn make_ghost_dir_with_debug_log(debug_log: bool) -> TempDir {
        let dir = tempfile::tempdir().expect("tempdir作成失敗");
        std::fs::write(
            dir.path().join("config.toml"),
            format!("[characters]\n\"湊\" = \"\\\\0\"\n\n[settings]\ndebug_log = {}\n", debug_log),
        ).expect("config.toml書き込み失敗");
        std::fs::create_dir(dir.path().join("talks")).expect("talksディレクトリ作成失敗");
        std::fs::write(
            dir.path().join("talks").join("main.mnt"),
            "OnBoot => {\n    湊: おはよう\n}\n",
        ).expect("main.mnt書き込み失敗");
        dir
    }


    #[test]
fn test_top_level_global_errors_reported_once() {
    let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    std::fs::write(
        dir.path().join("config.toml"),
        "[characters]\n\"湊\" = \"\\\\0\"\n",
    ).expect("config.toml書き込み失敗");
    std::fs::create_dir(dir.path().join("talks")).expect("talks作成失敗");
    std::fs::write(
        dir.path().join("talks").join("main.mnt"),
        "global save.x = 1 / 0\nOnBoot => {\n    湊: おはよう\n}\n",
    ).expect("main.mnt書き込み失敗");

    let state = init(dir.path()).expect("init失敗");
    assert!(!state.init_errors.is_empty(), "トップレベルglobalのエラーが回収されていない");
    assert!(state.codegen.errors.is_empty(), "codegen.errorsに残っている");

    if let Ok(mut s) = STATE.lock() { *s = Some(state); }

    let req = "SEND SHIORI/3.0\r\nID: OnBoot\r\nSender: SSP\r\nCharset: UTF-8\r\n\r\n";
    let res1 = handle_request(req);
    assert!(res1.contains("ErrorDescription"), "初回応答にエラーが載っていない");
    assert!(res1.contains("トップレベル"), "帰属が分かる文言が付いていない");

    let res2 = handle_request(req);
    assert!(!res2.contains("トップレベル"), "2回目にも初期化エラーが出ている");

    if let Ok(mut s) = STATE.lock() { *s = None; }
}

    #[test]
fn test_top_level_global_notice_is_not_reported() {
    let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    std::fs::write(
        dir.path().join("config.toml"),
        "[characters]\n\"湊\" = \"\\\\0\"\n",
    ).expect("config.toml書き込み失敗");
    std::fs::create_dir(dir.path().join("talks")).expect("talks作成失敗");
    // save.未定義キー への参照はnoticeを出すが、
    // ?= による初期化は台本として正常な書き方なので通知すべきでない
    std::fs::write(
        dir.path().join("talks").join("main.mnt"),
        "global save.x = system['存在しないキー']\nOnBoot => {\n    湊: おはよう\n}\n",
    ).expect("main.mnt書き込み失敗");

    let state = init(dir.path()).expect("init失敗");
    assert!(
        state.init_errors.is_empty(),
        "noticeがinit_errorsに残っている: {:?}",
        state.init_errors
    );

    if let Ok(mut s) = STATE.lock() { *s = Some(state); }
    let req = "SEND SHIORI/3.0\r\nID: OnBoot\r\nSender: SSP\r\nCharset: UTF-8\r\n\r\n";
    let res = handle_request(req);
    assert!(!res.contains("ErrorDescription"), "noticeが応答に載っている: {}", res);
    if let Ok(mut s) = STATE.lock() { *s = None; }
}
#[test]
fn test_top_level_global_cannot_override_persisted_system_setting() {
    let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    // 過去にユーザーがtalk_intervalを300に変更し、save.jsonに永続化された想定
    std::fs::write(
        dir.path().join("save.json"),
        r#"{"system":{"talk_interval":300}}"#,
    ).expect("save.json書き込み失敗");
    std::fs::write(
        dir.path().join("config.toml"),
        "[characters]\n\"湊\" = \"\\\\0\"\n\n[settings]\ntalk_interval_secs = 60\n",
    ).expect("config.toml書き込み失敗");
    std::fs::create_dir(dir.path().join("talks")).expect("talks作成失敗");
    // 台本側はconfig.tomlと同じ60を「初期値のつもり」で書いている
    std::fs::write(
        dir.path().join("talks").join("main.mnt"),
        "global system.talk_interval = 60\nOnBoot => {\n    湊: おはよう\n}\n",
    ).expect("main.mnt書き込み失敗");

    let state = init(dir.path()).expect("init失敗");

    // save.jsonの300が勝つべき（台本の60に上書きされてはいけない）
    assert_eq!(state.talk_interval_secs, 300, "台本のトップレベルglobalが永続設定を上書きしている");
}

#[test]
fn test_save_star_still_gets_default_fill_after_load() {
    let _g = TEST_GUARD.lock().unwrap_or_else(|e|e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    // save.jsonには古いフィールドだけあり、新しいフィールドは無い想定
    std::fs::write(
        dir.path().join("save.json"),
        r#"{"save":{"訪問回数":5}}"#,
    ).expect("save.json書き込み失敗");
    std::fs::write(
        dir.path().join("config.toml"),
        "[characters]\n\"湊\" = \"\\\\0\"\n",
    ).expect("config.toml書き込み失敗");
    std::fs::create_dir(dir.path().join("talks")).expect("talks作成失敗");
    // 台本更新で新フィールドを追加、?=で初期化する意図
    std::fs::write(
        dir.path().join("talks").join("main.mnt"),
        "global save.好感度 ?= 0\nOnBoot => {\n    湊: ${save.訪問回数}/${save.好感度}\n}\n",
    ).expect("main.mnt書き込み失敗");

    let state = init(dir.path()).expect("init失敗");
    match state.codegen.env.globals.get("save") {
        Some(codegen::Value::Map(m)) => {
            assert_eq!(m["訪問回数"].as_number(), 5.0, "既存フィールドが失われている");
            assert_eq!(m["好感度"].as_number(), 0.0, "新フィールドの?=初期化が効いていない");
        }
        other => panic!("saveが存在しない: {:?}", other),
    }
}


#[test]
fn test_status_header_reflected_in_talk() {
    let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    std::fs::write(
        dir.path().join("config.toml"),
        "[characters]\n\"湊\" = \"\\\\0\"\n",
    ).expect("config.toml書き込み失敗");
    std::fs::create_dir(dir.path().join("talks")).expect("talks作成失敗");
    std::fs::write(
        dir.path().join("talks").join("main.mnt"),
        "OnBoot => {\n    湊: ${status.talking}/${status.minimizing}\n}\n",
    ).expect("main.mnt書き込み失敗");

    let state = init(dir.path()).expect("init失敗");
    if let Ok(mut s) = STATE.lock() { *s = Some(state); }

    let req = "SEND SHIORI/3.0\r\nID: OnBoot\r\nSender: SSP\r\nCharset: UTF-8\r\nStatus: talking,balloon(0=2/1=0)\r\n\r\n";
    let res = handle_request(req);
    assert!(res.contains("true/false"), "Statusヘッダがtalking=trueとして反映されていない: {}", res);

    if let Ok(mut s) = STATE.lock() { *s = None; }
}

#[test]
fn test_status_header_absent_defaults_to_empty() {
    let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    std::fs::write(
        dir.path().join("config.toml"),
        "[characters]\n\"湊\" = \"\\\\0\"\n",
    ).expect("config.toml書き込み失敗");
    std::fs::create_dir(dir.path().join("talks")).expect("talks作成失敗");
    std::fs::write(
        dir.path().join("talks").join("main.mnt"),
        "OnBoot => {\n    湊: ${status.talking}\n}\n",
    ).expect("main.mnt書き込み失敗");

    let state = init(dir.path()).expect("init失敗");
    if let Ok(mut s) = STATE.lock() { *s = Some(state); }

    let req = "SEND SHIORI/3.0\r\nID: OnBoot\r\nSender: SSP\r\nCharset: UTF-8\r\n\r\n";
    let res = handle_request(req);
    assert!(res.contains("false"), "Statusヘッダ無しでtalkingがfalse以外になっている: {}", res);

    if let Ok(mut s) = STATE.lock() { *s = None; }
}



#[test]
fn test_status_header_cleared_when_absent_in_later_request() {
    let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    std::fs::write(
        dir.path().join("config.toml"),
        "[characters]\n\"湊\" = \"\\\\0\"\n",
    ).expect("config.toml書き込み失敗");
    std::fs::create_dir(dir.path().join("talks")).expect("talks作成失敗");
    std::fs::write(
        dir.path().join("talks").join("main.mnt"),
        "OnBoot => {\n    湊: ${status.talking}\n}\n",
    ).expect("main.mnt書き込み失敗");

    let state = init(dir.path()).expect("init失敗");
    if let Ok(mut s) = STATE.lock() { *s = Some(state); }

    let with_status = "SEND SHIORI/3.0\r\nID: OnBoot\r\nSender: SSP\r\nCharset: UTF-8\r\nStatus: talking\r\n\r\n";
    let res1 = handle_request(with_status);
    assert!(res1.contains("true"), "Statusありでtalkingがtrueになっていない: {}", res1);

    // Statusヘッダの無いリクエストでは前回の値が残ってはいけない
    let without_status = "SEND SHIORI/3.0\r\nID: OnBoot\r\nSender: SSP\r\nCharset: UTF-8\r\n\r\n";
    let res2 = handle_request(without_status);
    assert!(
        res2.contains("Value: \\0false"),
        "Statusヘッダが無いのに前回のtalking=trueが持ち越されている: {}",
        res2
    );

    if let Ok(mut s) = STATE.lock() { *s = None; }
}
    #[test]
    fn test_request_log_not_written_when_debug_log_off() {
        let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = make_ghost_dir_with_debug_log(false);
        let state = init(dir.path()).expect("init失敗");
        if let Ok(mut s) = STATE.lock() { *s = Some(state); }

        let req = "SEND SHIORI/3.0\r\nID: OnBoot\r\nSender: SSP\r\nCharset: UTF-8\r\n\r\n";
        let _ = handle_request(req);

        assert!(!dir.path().join("minato_request.log").exists(), "debug_log=falseなのにログファイルが作られている");

        if let Ok(mut s) = STATE.lock() { *s = None; }
    }

    #[test]
    fn test_request_log_written_when_debug_log_on() {
        let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = make_ghost_dir_with_debug_log(true);
        let state = init(dir.path()).expect("init失敗");
        if let Ok(mut s) = STATE.lock() { *s = Some(state); }

        let req = "SEND SHIORI/3.0\r\nID: OnBoot\r\nSender: SSP\r\nCharset: UTF-8\r\n\r\n";
        let _response = handle_request(req);

        assert!(dir.path().join("minato_request.log").exists(), "debug_log=trueなのにログファイルが作られていない");

        if let Ok(mut s) = STATE.lock() { *s = None; }
    }

    
#[test]
fn test_panic_bak_notice_is_reported_once_per_incident() {
    let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = make_ghost_dir_with_debug_log(false);
    std::fs::write(dir.path().join(PANIC_BAK), b"{}").unwrap();

    let s1 = init(dir.path()).expect("1回目のinit失敗");
    assert!(
        s1.init_errors.iter().any(|(_, m)| m.contains(PANIC_BAK)),
        "初回に退避の通知が出ていない: {:?}", s1.init_errors
    );
    assert!(dir.path().join(PANIC_BAK_NOTIFIED).exists(), "マーカーが作られていない");

    let s2 = init(dir.path()).expect("2回目のinit失敗");
    assert!(
        !s2.init_errors.iter().any(|(_, m)| m.contains(PANIC_BAK)),
        "2回目にも通知が出ている: {:?}", s2.init_errors
    );

    // 新しいインシデント相当（save_globalsがマーカーを消した状態）
    std::fs::remove_file(dir.path().join(PANIC_BAK_NOTIFIED)).unwrap();
    let s3 = init(dir.path()).expect("3回目のinit失敗");
    assert!(
        s3.init_errors.iter().any(|(_, m)| m.contains(PANIC_BAK)),
        "新しいインシデントで通知が出ていない: {:?}", s3.init_errors
    );
}

#[test]
fn test_panic_bak_notice_delivered_even_on_parse_error() {
    let _g = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(mut s) = STATE.lock() { *s = None; }

    let dir = tempfile::tempdir().expect("tempdir作成失敗");
    std::fs::write(
        dir.path().join("config.toml"),
        "[characters]\n\"湊\" = \"\\\\0\"\n",
    ).expect("config.toml書き込み失敗");
    std::fs::create_dir(dir.path().join("talks")).expect("talks作成失敗");
    // 「=>」を忘れた壊れた台本
    std::fs::write(
        dir.path().join("talks").join("main.mnt"),
        "OnBoot {\n    湊: おはよう\n}\n",
    ).expect("main.mnt書き込み失敗");

    std::fs::write(dir.path().join(PANIC_BAK), b"{}").unwrap();

    let state = init(dir.path()).expect("init失敗");
    assert!(state.parse_error.is_some(), "パースエラー状態になっていない");
    assert!(
        state.init_errors.iter().any(|(_, m)| m.contains(PANIC_BAK)),
        "パースエラー経路でinit_errorsが捨てられている: {:?}", state.init_errors
    );

    if let Ok(mut s) = STATE.lock() { *s = Some(state); }

    let req = "SEND SHIORI/3.0\r\nID: OnBoot\r\nSender: SSP\r\nCharset: UTF-8\r\n\r\n";
    let res = handle_request(req);
    assert!(res.contains("ErrorDescription"), "エラーが応答に載っていない: {}", res);
    assert!(res.contains(PANIC_BAK), "退避の通知が届いていない: {}", res);
    assert!(res.contains("パースエラー"), "パースエラー本体が届いていない: {}", res);

    // 2回目には出ない（takeが効いている）
    let res2 = handle_request(req);
    assert!(!res2.contains(PANIC_BAK), "2回目にも通知が出ている: {}", res2);

    if let Ok(mut s) = STATE.lock() { *s = None; }
}
}

