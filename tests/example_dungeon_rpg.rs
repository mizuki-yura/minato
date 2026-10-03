// examples/dungeon_rpg（ダンジョンRPGのサンプル）を実際のDLLエントリポイント
// （loadu / request / unload）で動かし、サンプルが今の湊でそのまま動くことを確かめる。
// 湊の仕様変更でサンプルだけが古くなるのを防ぐためのテスト。
//
// サンプルのファイルをそのまま (ホーム)/ghost/master に置き、乱数に頼らず
// 戦闘や持ち物を確かめるためのイベントだけをテスト側で main.mnt に書き足す。

use std::ffi::c_long;
use std::fs;
use std::path::Path;

use encoding_rs::SHIFT_JIS;
use winapi::shared::minwindef::HGLOBAL;
use winapi::um::winbase::{GlobalAlloc, GlobalFree, GMEM_FIXED};

const GET_PROPERTY_SUFFIX_HEAD: &str = "\\![get,property,OnGotVirtualTime,";
const MENU: &str = "\\q[探索する,OnExplore]\\q[持ち物,OnBag]\\q[店,OnShop]";
const BATTLE: &str = "\\q[戦う,OnAttack]\\q[逃げる,OnMenu]";

const TEST_EVENTS: &str = "
OnTestBattle => {
    start_battle(reference[\"0\"])
    call 戦闘開始
}

OnTestGive => {
    bag_add(reference[\"0\"])
}

OnTestHp => {
    global save.hp = 1
}
";

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

/// refは選択肢の値（Reference0）。無ければ空文字列を渡す。
fn send(id: &str, r#ref: &str) -> String {
    let mut req = format!("GET SHIORI/3.0\r\nCharset: Shift_JIS\r\nSender: SSP\r\nID: {}\r\n", id);
    if !r#ref.is_empty() {
        req += &format!("Reference0: {}\r\n", r#ref);
    }
    req += "\r\n";
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

fn talk(id: &str, r#ref: &str) -> String {
    value_of(id, &send(id, r#ref))
}

fn say(text: &str, choices: &str) -> String {
    format!("\\0\\s[0]{}{}\\e", text, choices)
}

/// サンプルを (ホーム)/ghost/master に置く。itemsを渡すとitems.jsonをその内容にする。
fn setup(home: &Path, items: Option<&str>) -> std::path::PathBuf {
    let ex = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/dungeon_rpg/ghost/master");
    let master = home.join("ghost").join("master");
    fs::create_dir_all(master.join("talks")).unwrap();
    for f in ["config.toml", "items.json", "monsters.json", "talks/rpg.mnt"] {
        fs::copy(ex.join(f), master.join(f)).unwrap();
    }
    if let Some(text) = items {
        fs::write(master.join("items.json"), text).unwrap();
    }
    let main = fs::read_to_string(ex.join("talks/main.mnt")).unwrap() + TEST_EVENTS;
    fs::write(master.join("talks/main.mnt"), main).unwrap();
    master
}

fn saved(master: &Path) -> serde_json::Value {
    let text = fs::read_to_string(master.join("save.json")).unwrap();
    serde_json::from_str::<serde_json::Value>(&text).unwrap()["save"].clone()
}

// DLLの状態はプロセスで1つなので、読み込み失敗の確認も同じテストの中で順に行う。
#[test]
fn example_dungeon_rpg_works() {
    let home = tempfile::tempdir().unwrap();
    let master = setup(home.path(), None);
    assert_eq!(load(&master), 1, "loaduに失敗");

    assert_eq!(talk("OnBoot", ""), say("ダンジョンの入口に着いたよ。\\w9HP30/30、20ゴールド。", MENU));

    // 店の品ぞろえはitems.jsonの順
    assert_eq!(
        talk("OnShop", ""),
        say(
            "いらっしゃい。\\w9（所持金20ゴールド）",
            "\\q[薬草（10G）,OnBuy,薬草]\\q[上薬草（30G）,OnBuy,上薬草]\\q[銅の剣（40G）,OnBuy,銅の剣]\\q[鉄の剣（120G）,OnBuy,鉄の剣]\\q[戻る,OnMenu]"
        )
    );
    assert_eq!(talk("OnBuy", "薬草"), say("薬草を買った。\\w9（残り10ゴールド）", MENU));
    assert_eq!(talk("OnBuy", "鉄の剣"), say("お金が足りないよ。\\w9（120ゴールド必要）", MENU));

    // HPが満タンなら薬草は減らない
    assert_eq!(talk("OnUse", "薬草"), say("HPは減ってないよ。", MENU));

    // スライム（HP8・攻撃3）を攻撃力3で3回たたく
    assert_eq!(talk("OnTestBattle", "スライム"), say("スライムが現れた！（HP8）", BATTLE));
    assert_eq!(talk("OnAttack", ""), say("スライムに3のダメージ！（残りHP5）\\w93のダメージを受けた。（HP27/30）", BATTLE));
    assert_eq!(talk("OnAttack", ""), say("スライムに3のダメージ！（残りHP2）\\w93のダメージを受けた。（HP24/30）", BATTLE));
    assert_eq!(talk("OnAttack", ""), say("スライムに3のダメージ！\\w9倒した！\\w95ゴールド手に入れた。薬草を拾った。", MENU));

    assert_eq!(talk("OnUse", "薬草"), say("薬草を使った。\\w9HPが6回復した。（HP30/30）", MENU));
    assert_eq!(talk("OnUse", "銅の剣"), say("銅の剣は持ってないよ。", MENU));

    assert!(send("OnTestGive", "銅の剣").contains("204 No Content"));
    assert_eq!(talk("OnUse", "銅の剣"), say("銅の剣を装備した。\\w9攻撃力が6になった。", MENU));
    assert_eq!(
        talk("OnBag", ""),
        say("何を使う？（HP30/30、攻撃力6）", "\\q[薬草×1,OnUse,薬草]\\q[銅の剣×1,OnUse,銅の剣]\\q[戻る,OnMenu]")
    );

    // 倒れると所持金が半分になり、HPは戻る
    assert!(send("OnTestHp", "").contains("204 No Content"));
    assert_eq!(talk("OnTestBattle", "コウモリ"), say("コウモリが現れた！（HP12）", BATTLE));
    assert_eq!(
        talk("OnAttack", ""),
        say("コウモリに6のダメージ！\\w9……でもやられちゃった。\\w9お金を半分なくして入口に戻ったよ。", MENU)
    );

    // 探索は乱数で結果が変わるので、警告なしで3通りのどれかになることだけ確かめる
    for _ in 0..30 {
        let v = talk("OnExplore", "");
        assert!(v.ends_with(&format!("{}\\e", MENU)) || v.ends_with(&format!("{}\\e", BATTLE)), "OnExplore: {}", v);
    }

    // 主人公の状態はsave.jsonに残り、次の起動で続きから遊べる
    minato::unload();
    let s = saved(&master);
    assert_eq!(s["weapon"], "銅の剣");
    assert_eq!(s["max_hp"], 30);
    let gold = s["gold"].as_i64().unwrap();
    let hp = s["hp"].as_i64().unwrap();
    assert_eq!(load(&master), 1, "2回目のloaduに失敗");
    assert_eq!(
        talk("OnBoot", ""),
        say(&format!("ダンジョンの入口に着いたよ。\\w9HP{}/30、{}ゴールド。", hp, gold), MENU)
    );
    minato::unload();

    // items.jsonが壊れていたら、メニューを出さずに知らせる（警告も出る）
    let home2 = tempfile::tempdir().unwrap();
    let master2 = setup(home2.path(), Some("{ \"薬草\": { \"値段\": 10, } }"));
    assert_eq!(load(&master2), 1, "loaduに失敗");
    let resp = send("OnBoot", "");
    assert!(resp.contains("ErrorDescription:"), "警告が出ていない: {}", resp);
    assert!(resp.contains("Value: \\0\\s[0]アイテム表かモンスター表が読めなかったよ。"), "{}", resp);
    minato::unload();
}
