// 移行ガイド（docs/src/migration/*.md）のサンプルコードを実際のDLLエントリポイント
// （loadu / request / unload）で実行し、ガイドに書かれた出力と一致するか検証する。
//
// ガイド側の書式:
//
//   <!--run OnBoot OnClose-->
//   ```mnt
//   （main.mnt の内容）
//   ```
//   ```text
//   （イベントごとに1行ずつ、SHIORIの Value）
//   ```
//
// - `<!--run ...-->` にはイベントIDを空白区切りで並べる。
//     OnMouseClick[3|Head]  … Reference0=3, Reference1=Head を付けて送る
//     @reload               … unload → loadu（再起動）
//     @save                 … その時点の save.json の "save" を1行のJSONで出力
// - 応答が 204 のときは「(204)」と書く。
// - OnBoot / OnMinuteChange の応答末尾に湊が付け足す
//   `\![get,property,OnGotVirtualTime,...]` は取り除いて比較する。
// - config.toml は湊(\0)と助手(\1)を登録した固定内容。
//
// STATEがプロセス全体で1つなので、全サンプルを1つのテスト関数で直列に実行する。

use std::ffi::c_long;
use std::fs;
use std::path::{Path, PathBuf};

use encoding_rs::SHIFT_JIS;
use winapi::shared::minwindef::HGLOBAL;
use winapi::um::winbase::{GlobalAlloc, GlobalFree, GMEM_FIXED};

const CONF: &str = "[characters]\n\"湊\" = \"\\\\0\"\n\"助手\" = \"\\\\1\"\n";
const GET_PROPERTY_SUFFIX_HEAD: &str = "\\![get,property,OnGotVirtualTime,";

fn alloc(bytes: &[u8]) -> HGLOBAL {
    unsafe {
        let mem = GlobalAlloc(GMEM_FIXED, bytes.len() + 1);
        let p = mem as *mut u8;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        *p.add(bytes.len()) = 0;
        mem
    }
}

fn load(dir: &Path) -> i32 {
    let s = dir.to_string_lossy().to_string();
    let h = alloc(s.as_bytes());
    minato::loadu(h, s.len() as c_long)
}

fn send(id: &str, refs: &[String]) -> String {
    let mut req = format!("GET SHIORI/3.0\r\nCharset: Shift_JIS\r\nSender: SSP\r\nID: {}\r\n", id);
    for (i, r) in refs.iter().enumerate() {
        req.push_str(&format!("Reference{}: {}\r\n", i, r));
    }
    req.push_str("\r\n");
    let (enc, _, _) = SHIFT_JIS.encode(&req);
    let h = alloc(&enc);
    let mut len = enc.len() as c_long;
    let out = minato::request(h, &mut len);
    if out.is_null() {
        return "<null>".to_string();
    }
    let bytes = unsafe { std::slice::from_raw_parts(out as *const u8, len as usize) }.to_vec();
    unsafe { GlobalFree(out) };
    SHIFT_JIS.decode(&bytes).0.into_owned()
}

/// 応答からValueを取り出す。204は「(204)」。
fn value_of(resp: &str) -> String {
    if resp.starts_with("SHIORI/3.0 204") {
        return "(204)".to_string();
    }
    for l in resp.lines() {
        if let Some(v) = l.strip_prefix("Value: ") {
            let v = match v.find(GET_PROPERTY_SUFFIX_HEAD) {
                Some(i) => format!("{}\\e", &v[..i]),
                None => v.to_string(),
            };
            return v;
        }
    }
    format!("(no Value: {})", resp.lines().next().unwrap_or(""))
}

struct Sample {
    file: String,
    line: usize,
    events: Vec<String>,
    mnt: String,
    /// 起動前に置く save.json（`<!--run-->` の直後の ```json ブロック）
    seed: Option<String>,
    expected: Vec<String>,
    /// `<!--any EVENT-->` 形式。EVENTを繰り返し送り、応答が期待行のどれかに一致し、
    /// かつ期待行が全て1回以上現れることを検証する（ランダム選択のサンプル用）。
    any: bool,
    /// `<!--anyof ...-->` のときfalse。出力が期待行のどれかに一致することだけを検証し、
    /// 全部が現れることまでは求めない。
    cover: bool,
}

fn extract_samples(path: &Path) -> Vec<Sample> {
    let text = fs::read_to_string(path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i].trim();
        let (marker, any, cover) = if l.starts_with("<!--anyof") {
            (l.strip_prefix("<!--anyof"), true, false)
        } else if l.starts_with("<!--any") {
            (l.strip_prefix("<!--any"), true, true)
        } else {
            (l.strip_prefix("<!--run"), false, true)
        };
        if let Some(rest) = marker {
            let events: Vec<String> = rest
                .trim_end_matches("-->")
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            let start_line = i + 1;
            // ```json（あれば save.json の初期値）と ```mnt を読み、```text の手前まで進める
            i += 1;
            let mut mnt = String::new();
            let mut seed: Option<String> = None;
            while i < lines.len() && !lines[i].trim_start().starts_with("```text") {
                let t = lines[i].trim_start();
                if t.starts_with("```mnt") || t.starts_with("```json") {
                    let is_json = t.starts_with("```json");
                    i += 1;
                    let mut buf = String::new();
                    while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                        buf.push_str(lines[i]);
                        buf.push('\n');
                        i += 1;
                    }
                    if is_json { seed = Some(buf); } else { mnt = buf; }
                }
                i += 1;
            }
            i += 1;
            let mut expected = Vec::new();
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                expected.push(lines[i].to_string());
                i += 1;
            }
            out.push(Sample {
                file: path.file_name().unwrap().to_string_lossy().to_string(),
                line: start_line,
                events,
                mnt,
                seed,
                expected,
                any,
                cover,
            });
        }
        i += 1;
    }
    out
}

fn parse_event(tok: &str) -> (String, Vec<String>) {
    match tok.find('[') {
        Some(i) => {
            let id = tok[..i].to_string();
            let refs = tok[i + 1..tok.len() - 1].split('|').map(|s| s.to_string()).collect();
            (id, refs)
        }
        None => (tok.to_string(), vec![]),
    }
}

fn save_json_compact(dir: &Path) -> String {
    let s = fs::read_to_string(dir.join("save.json")).unwrap_or_default();
    match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(v) => v.get("save").map(|x| x.to_string()).unwrap_or_else(|| "{}".to_string()),
        Err(_) => "(save.json なし)".to_string(),
    }
}

/// `*` が任意の文字列にマッチする簡易グロブ（期待行に `*` を書けるようにする）
fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            if !rest.starts_with(part) { return false; }
            rest = &rest[part.len()..];
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            match rest.find(part) {
                Some(p) => rest = &rest[p + part.len()..],
                None => return false,
            }
        }
    }
    true
}

struct MasterDir(PathBuf);
impl MasterDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

fn run_sample(s: &Sample) -> Result<(), String> {
    // 本番と同じ (ゴーストのホーム)/ghost/master 構成にする。
    // file_read/file_write のパスはホームからの相対パスで、書き込みは ghost/master 配下限定。
    let home = tempfile::tempdir().unwrap();
    let master = home.path().join("ghost").join("master");
    fs::create_dir_all(master.join("talks")).unwrap();
    fs::write(master.join("config.toml"), CONF).unwrap();
    fs::write(master.join("talks").join("main.mnt"), &s.mnt).unwrap();
    if let Some(seed) = &s.seed {
        fs::write(master.join("save.json"), seed).unwrap();
    }
    let dir = MasterDir(master);
    if load(dir.path()) != 1 {
        minato::unload();
        return Err("loadu が 0 を返しました".to_string());
    }
    if s.any {
        let (id, refs) = parse_event(&s.events[0]);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..90 {
            let v = value_of(&send(&id, &refs));
            if !s.expected.iter().any(|p| glob_match(p, &v)) {
                minato::unload();
                return Err(format!("許容されない出力: {}\n  許容:\n{}", v,
                    s.expected.iter().map(|x| format!("    {}", x)).collect::<Vec<_>>().join("\n")));
            }
            seen.insert(v);
        }
        minato::unload();
        let missing: Vec<&String> = s.expected.iter().filter(|e| !seen.contains(*e)).collect();
        return if !s.cover || missing.is_empty() { Ok(()) } else { Err(format!("90回試しても現れなかった出力: {:?}", missing)) };
    }
    let mut actual = Vec::new();
    for tok in &s.events {
        if tok == "@reload" {
            minato::unload();
            if load(dir.path()) != 1 {
                return Err("再ロードに失敗".to_string());
            }
            actual.push("(reload)".to_string());
        } else if tok == "@save" {
            minato::unload(); // 保存を確定させる
            actual.push(save_json_compact(dir.path()));
            if load(dir.path()) != 1 {
                return Err("再ロードに失敗".to_string());
            }
        } else {
            let (id, refs) = parse_event(tok);
            actual.push(value_of(&send(&id, &refs)));
        }
    }
    minato::unload();
    if actual == s.expected {
        Ok(())
    } else {
        Err(format!(
            "出力が一致しません\n  期待:\n{}\n  実際:\n{}",
            s.expected.iter().map(|x| format!("    {}", x)).collect::<Vec<_>>().join("\n"),
            actual.iter().map(|x| format!("    {}", x)).collect::<Vec<_>>().join("\n"),
        ))
    }
}

fn guide_dir() -> PathBuf {
    // 作業中の下書きを検証したいとき用（既定は docs/src/migration）
    if let Ok(d) = std::env::var("MIGRATION_GUIDE_DIR") {
        return PathBuf::from(d);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join("src").join("migration")
}

#[test]
fn guide_samples_match_real_behavior() {
    let mut files: Vec<PathBuf> = fs::read_dir(guide_dir())
        .expect("docs/src/migration がありません")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map_or(false, |x| x == "md"))
        .collect();
    files.sort();

    let mut total = 0;
    let mut failures = Vec::new();
    for f in &files {
        for s in extract_samples(f) {
            total += 1;
            if let Err(msg) = run_sample(&s) {
                failures.push(format!("{}:{} — {}", s.file, s.line, msg));
            }
        }
    }
    println!("検証したサンプル数: {}", total);
    assert!(total > 0, "検証対象のサンプルが1つもありません");
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n\n"));
}
