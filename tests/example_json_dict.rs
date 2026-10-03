// examples/json_dict（JSONで辞書を管理するサンプル）を実際のDLLエントリポイント
// （loadu / request / unload）で動かし、サンプルが今の湊でそのまま動くことを確かめる。
// 湊の仕様変更でサンプルだけが古くなるのを防ぐためのテスト。
//
// サンプルのdict.mnt・dict.json・main.mntをそのまま (ホーム)/ghost/master に置き、
// 言葉の削除を確かめるイベントだけをテスト側で main.mnt に書き足す。

use std::ffi::c_long;
use std::fs;
use std::path::Path;

use encoding_rs::SHIFT_JIS;
use winapi::shared::minwindef::HGLOBAL;
use winapi::um::winbase::{GlobalAlloc, GlobalFree, GMEM_FIXED};

const CONF: &str = "[characters]\n\"うきわ君\" = '\\0'\n";
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

fn send(id: &str) -> String {
    let req = format!("GET SHIORI/3.0\r\nCharset: Shift_JIS\r\nSender: SSP\r\nID: {}\r\n\r\n", id);
    let (enc, _, _) = SHIFT_JIS.encode(&req);
    let h = alloc(&enc);
    let mut len = enc.len() as c_long;
    let out = minato::request(h, &mut len);
    assert!(!out.is_null(), "{} の応答がnull", id);
    let bytes = unsafe { std::slice::from_raw_parts(out as *const u8, len as usize) }.to_vec();
    unsafe { GlobalFree(out) };
    SHIFT_JIS.decode(&bytes).0.into_owned()
}

/// 応答からValueを取り出す。警告やエラーが付いていたら失敗にする
/// （サンプルは警告なしで動くべきなので）。
fn value_of(id: &str, resp: &str) -> String {
    assert!(!resp.contains("ErrorDescription:"), "{} で警告・エラーが出た: {}", id, resp);
    let v = resp.lines()
        .find_map(|l| l.strip_prefix("Value: "))
        .unwrap_or_else(|| panic!("{} の応答にValueが無い: {}", id, resp));
    match v.find(GET_PROPERTY_SUFFIX_HEAD) {
        Some(i) => format!("{}\\e", &v[..i]),
        None => v.to_string(),
    }
}

fn topics(master: &Path) -> Vec<String> {
    let text = fs::read_to_string(master.join("dict.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json["話題"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

#[test]
fn example_json_dict_works() {
    let ex = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/json_dict/ghost/master");
    let home = tempfile::tempdir().unwrap();
    let master = home.path().join("ghost").join("master");
    fs::create_dir_all(master.join("talks")).unwrap();
    fs::write(master.join("config.toml"), CONF).unwrap();
    fs::copy(ex.join("dict.json"), master.join("dict.json")).unwrap();
    fs::copy(ex.join("talks/dict.mnt"), master.join("talks/dict.mnt")).unwrap();
    let main = fs::read_to_string(ex.join("talks/main.mnt")).unwrap()
        + "\nOnForget => {\n    [0]うきわ君:${dict_remove(\"話題\", \"海に行きたいな。\")}|${dict_remove(\"話題\", \"無い言葉\")}|${dict_count(\"話題\")}\n}\n";
    fs::write(master.join("talks/main.mnt"), main).unwrap();

    assert_eq!(load(&master), 1, "loaduに失敗");

    let boot = value_of("OnBoot", &send("OnBoot"));
    assert!(
        boot == "\\0\\s[0]やあ。\\w9今日も開発していこう。\\w9\\e"
            || boot == "\\0\\s[0]こんにちは。\\w9今日も開発していこう。\\w9\\e",
        "OnBoot: {}", boot
    );

    let talk = value_of("OnRandomTalk", &send("OnRandomTalk"));
    assert!(
        talk == "\\0\\s[0]今日はいい天気だね。\\e" || talk == "\\0\\s[0]お茶でも飲もうか。\\e",
        "OnRandomTalk: {}", talk
    );

    // 覚える → dict.jsonに書き込まれる
    assert_eq!(value_of("OnRemember", &send("OnRemember")), "\\0\\s[0]新しい話題を覚えたよ。\\w9いま3個。\\e");
    assert_eq!(topics(&master), ["今日はいい天気だね。", "お茶でも飲もうか。", "海に行きたいな。"]);

    // 同じ言葉は二重に覚えない
    assert_eq!(value_of("OnRemember", &send("OnRemember")), "\\0\\s[0]それはもう知ってるよ。\\e");
    assert_eq!(topics(&master).len(), 3);

    // 削除 → dict.jsonから消える。無い言葉はfalse
    assert_eq!(value_of("OnForget", &send("OnForget")), "\\0\\s[0]true|false|2\\e");
    assert_eq!(topics(&master), ["今日はいい天気だね。", "お茶でも飲もうか。"]);

    // 他の分類とキー順はそのまま
    let text = fs::read_to_string(master.join("dict.json")).unwrap();
    assert!(text.find("\"挨拶\"").unwrap() < text.find("\"話題\"").unwrap(), "{}", text);

    minato::unload();
}
