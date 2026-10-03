// json.rs - 台本の値（codegen::Value）とJSONテキストの相互変換
//
// save.jsonの保存・読み込みと、台本のjson_parse / json_stringifyで共用する。
// どちらも同じ変換規則を通すことで、「save.jsonに書けた値はjson_stringifyでも
// 同じ形で書ける」ことを保証する。
// warningの積み方は呼び出し側の事情（台本実行中か、unload中か）で違うので、
// ここではResultで失敗を返すだけにしている。

use indexmap::IndexMap;

use crate::codegen::{Value, FILE_READ_LIMIT};

/// value_to_jsonが辿る入れ子の深さの上限。
/// 台本がループで深いMapを作ると、再帰で変換するvalue_to_jsonが
/// スタックを溢れさせ、DLLごと落ちる（catch_unwindでは捕まえられない）。
/// serde_jsonがパース時に使う再帰上限（128段）より小さくして、
/// 「書き出せる値は必ず読み戻せる」ようにしている。
pub const MAX_DEPTH: usize = 100;

/// json_stringifyの出力サイズの上限。file_readの上限と揃えることで、
/// 書き出したJSONを必ずfile_readで読み戻せるようにしている。
pub const MAX_OUTPUT: usize = FILE_READ_LIMIT as usize;

/// f64で誤差なく表せる整数の上限（2^53）。これを超える整数を
/// 「整数として」書くと、元の値と違う桁が出力されてしまう。
const MAX_SAFE_INT: f64 = 9_007_199_254_740_992.0;

pub fn json_to_value(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::Str(s),
        serde_json::Value::Array(a) => Value::Array(a.into_iter().map(json_to_value).collect()),
        // preserve_orderによりserde_json::Mapは挿入順を保つので、
        // IndexMapへ移してもJSONファイル上のキー順が維持される
        serde_json::Value::Object(o) => Value::Map(
            o.into_iter()
                .map(|(k, v)| (k, json_to_value(v)))
                .collect::<IndexMap<_, _>>()
        ),
    }
}

/// 深さがMAX_DEPTHを超えたらErrを返す。
pub fn value_to_json(v: &Value) -> Result<serde_json::Value, String> {
    value_to_json_at(v, 0)
}

fn value_to_json_at(v: &Value, depth: usize) -> Result<serde_json::Value, String> {
    if depth > MAX_DEPTH {
        return Err(format!("値の入れ子が深すぎます（上限{}段）", MAX_DEPTH));
    }
    Ok(match v {
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::Number(n) => number_to_json(*n),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Array(a) => serde_json::Value::Array(
            a.iter().map(|x| value_to_json_at(x, depth + 1)).collect::<Result<_, _>>()?
        ),
        Value::Map(m) => serde_json::Value::Object(
            m.iter()
                .map(|(k, x)| Ok((k.clone(), value_to_json_at(x, depth + 1)?)))
                .collect::<Result<_, String>>()?
        ),
        Value::Null => serde_json::Value::Null,
    })
}

/// 台本の数値はすべてf64だが、そのままserde_jsonに渡すと100が「100.0」と
/// 書かれる。外部のアイテム表を読んで書き戻しただけで見た目が変わるのを
/// 避けるため、誤差なく整数で表せる値は整数として書く。
/// NaN / InfinityはJSONで表せないのでnullになる。
fn number_to_json(n: f64) -> serde_json::Value {
    if n.is_finite() && n.fract() == 0.0 && n.abs() <= MAX_SAFE_INT {
        serde_json::Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null)
    }
}

/// 失敗時のメッセージはErrorDescriptionヘッダに入るので改行を含めない。
pub fn parse(src: &str) -> Result<Value, String> {
    // メモ帳などで保存したJSONは先頭にBOMが付くことがあり、
    // serde_jsonはそれを「不正な文字」として拒否するので取り除く
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    serde_json::from_str::<serde_json::Value>(src)
        .map(json_to_value)
        .map_err(|e| {
            use serde_json::error::Category;
            let reason = match e.classify() {
                Category::Eof => "途中で終わっています",
                Category::Syntax => "書き方が正しくありません",
                Category::Data | Category::Io => "値が正しくありません",
            };
            format!("JSONの読み込みに失敗しました（{}行目{}文字目: {}）", e.line(), e.column(), reason)
        })
}

/// prettyがtrueなら字下げ付きで、falseなら1行で書く。
pub fn stringify(v: &Value, pretty: bool) -> Result<String, String> {
    let json = value_to_json(v)?;
    let mut w = LimitedWriter { buf: Vec::new(), limit: MAX_OUTPUT };
    let res = if pretty {
        serde_json::to_writer_pretty(&mut w, &json)
    } else {
        serde_json::to_writer(&mut w, &json)
    };
    match res {
        Ok(()) => Ok(String::from_utf8(w.buf).expect("serde_jsonはUTF-8を出力する")),
        Err(_) => Err(format!("JSONが大きすぎます（上限{}バイト）", MAX_OUTPUT)),
    }
}

/// 上限を超えた時点で書き込みを打ち切るWriter。
/// 全部書き終えてから長さを調べると、巨大な値ではその前に
/// メモリを使い切ってしまうため、書きながら止める。
struct LimitedWriter {
    buf: Vec<u8>,
    limit: usize,
}

impl std::io::Write for LimitedWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.buf.len() + data.len() > self.limit {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "too large"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested_array(depth: usize) -> Value {
        let mut v = Value::Null;
        for _ in 0..depth { v = Value::Array(vec![v]); }
        v
    }

    #[test]
    fn test_parse_strips_bom() {
        let v = parse("\u{feff}{\"a\": 1}").expect("BOM付きでも読めるべき");
        match v {
            Value::Map(m) => assert!(matches!(m.get("a"), Some(Value::Number(n)) if *n == 1.0)),
            _ => panic!("Mapになるべき: {:?}", v),
        }
    }

    #[test]
    fn test_parse_broken_json_is_err_single_line() {
        let e = parse("{\"a\": 1,").unwrap_err();
        assert!(e.contains("JSONの読み込みに失敗しました"), "{}", e);
        assert!(!e.contains('\n'), "改行を含んではいけない: {:?}", e);
        let e = parse("{a: 1}").unwrap_err();
        assert!(e.contains("1行目"), "{}", e);
    }

    #[test]
    fn test_parse_valid_null_is_ok() {
        assert!(matches!(parse("null"), Ok(Value::Null)));
    }

    #[test]
    fn test_parse_keeps_key_order() {
        let v = parse(r#"{"z":1,"a":2,"m":3,"b":4}"#).unwrap();
        let Value::Map(m) = v else { panic!() };
        let keys: Vec<&str> = m.keys().map(|s| s.as_str()).collect();
        assert_eq!(keys, vec!["z", "a", "m", "b"]);
    }

    #[test]
    fn test_parse_too_deep_input_is_err() {
        // serde_jsonの再帰上限で弾かれ、json_to_valueの再帰に到達しない
        let src = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
        assert!(parse(&src).is_err());
    }

    #[test]
    fn test_japanese_keys_roundtrip() {
        let src = r#"{"薬草":{"値段":10,"説明":"HPを\"少し\"回復"}}"#;
        let v = parse(src).unwrap();
        assert_eq!(stringify(&v, false).unwrap(), src);
    }

    #[test]
    fn test_roundtrip_nested() {
        let src = r#"{"items":[{"id":1,"hp":100,"rate":0.5,"rare":true,"memo":null}],"n":-3}"#;
        let v = parse(src).unwrap();
        assert_eq!(stringify(&v, false).unwrap(), src);
        // 整形して書いたものも同じ値として読み戻せる
        let pretty = stringify(&v, true).unwrap();
        assert!(pretty.contains('\n'));
        assert_eq!(stringify(&parse(&pretty).unwrap(), false).unwrap(), src);
    }

    #[test]
    fn test_integers_are_written_without_fraction() {
        assert_eq!(stringify(&Value::Number(100.0), false).unwrap(), "100");
        assert_eq!(stringify(&Value::Number(-0.0), false).unwrap(), "0");
        assert_eq!(stringify(&Value::Number(1.5), false).unwrap(), "1.5");
        // 2^53を超えると整数として正確に書けないので小数表記のまま
        assert_eq!(stringify(&Value::Number(1e20), false).unwrap(), "1e+20");
    }

    #[test]
    fn test_nan_and_infinity_become_null() {
        for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(stringify(&Value::Number(n), false).unwrap(), "null", "{}", n);
        }
    }

    #[test]
    fn test_too_deep_value_is_err() {
        assert!(stringify(&nested_array(MAX_DEPTH), false).is_ok());
        let e = stringify(&nested_array(MAX_DEPTH + 1), false).unwrap_err();
        assert!(e.contains("深すぎます"), "{}", e);
        // 書き出せる深さの値は読み戻せる
        let s = stringify(&nested_array(MAX_DEPTH), false).unwrap();
        assert!(parse(&s).is_ok());
    }

    #[test]
    fn test_too_large_output_is_err() {
        let big = Value::Str("a".repeat(MAX_OUTPUT));
        let e = stringify(&big, false).unwrap_err();
        assert!(e.contains("大きすぎます"), "{}", e);
        let ok = Value::Str("a".repeat(MAX_OUTPUT - 2));
        assert_eq!(stringify(&ok, false).unwrap().len(), MAX_OUTPUT);
    }
}
