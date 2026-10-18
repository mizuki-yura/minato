// config.rs
// config.toml の読み込み

use std::collections::HashMap;
use std::path::Path;
use serde::Deserialize;

// ── 設定構造体 ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Config {
    /// キャラクター名 → SAKURAスクリプトタグ
    /// 例: シャロン = "\0"
    #[serde(default)]
    pub characters: HashMap<String, String>,

    /// 全体設定
    #[serde(default)]
    pub settings: Settings,
}
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct Settings {
    #[serde(default = "default_true")]
    pub shuffle_reset: bool,

    #[serde(default = "default_encoding")]
    pub encoding: String,

    #[serde(default = "default_talk_interval")]
    pub talk_interval_secs: u64,

    #[serde(default = "default_talk_jitter")]
    pub talk_jitter_secs: u64,

    #[serde(default)]  // デフォルトはfalse
    pub debug_log: bool,

    
#[serde(default = "default_auto_newline")]
pub auto_newline: bool,
}

fn default_auto_newline() -> bool{ true }
impl Default for Settings {
    fn default() -> Self {
        Self {
            shuffle_reset: true,
            encoding: "UTF-8".to_string(),
            talk_interval_secs: 300,
            talk_jitter_secs: 180,
            debug_log: false,
            auto_newline: default_auto_newline(),
        }
    }
}


fn default_true() -> bool { true }
fn default_encoding() -> String { "UTF-8".to_string() }

/// encoding の値を「UTF-8か」に変換する。知らない値ならNone。
/// 綴りの揺れは file_read / file_write の文字コード指定に合わせる。
fn parse_encoding(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "utf-8" | "utf8" => Some(true),
        "shift_jis" | "shift-jis" | "sjis" => Some(false),
        _ => None,
    }
}

impl Settings {
    /// SSPへの応答をUTF-8で返すか。値はConfig::loadで確かめ済みなので、
    /// 知らない値は来ないが、来たらUTF-8にする。
    pub fn response_is_utf8(&self) -> bool {
        parse_encoding(&self.encoding).unwrap_or(true)
    }
}
fn default_talk_interval() -> u64 {300}
fn default_talk_jitter() -> u64 {180}
// ── 読み込み ─────────────────────────────────────────────
// 変更後
impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("config.toml が読めません: {}", e))?;
        let src: String = src.lines().collect::<Vec<_>>().join("\n");
        let config: Config = toml::from_str(&src)
            .map_err(|e| format!("config.toml のパースエラー: {}", e))?;
        if parse_encoding(&config.settings.encoding).is_none() {
            return Err(format!(
                "config.toml の encoding は \"UTF-8\" か \"Shift_JIS\" にしてください（今は「{}」）",
                config.settings.encoding
            ));
        }

        // 以前はここでnormalize_tag()によるエスケープ「正規化」処理を挟んでいたが、
        // 実装を精査した結果、その内部の置換ロジックが実質的なno-opであることが判明し、
        // TOMLパーサー自身のエスケープ規則（\\0 → \0）だけで意図した変換は既に
        // 完了していたため削除した。characters.toValues()の値はtoml crateが
        // パースした時点で最終形になっている。

        Ok(config)
    }
}




// ── テスト ───────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
// 変更後
fn parse(src: &str) -> Config {
    let src: String = src.chars().filter(|c| *c != '\r').collect();
    let config: Config = toml::from_str(&src).unwrap();
    config
}
#[test]
fn test_basic() {
 let _src = "[characters]\nshalon = \"\\\\0\"\nmurdock = \"\\\\1\"\n\n[settings]\nshuffle_reset = true\nencoding = \"UTF-8\"\n";
}
    
#[test]
fn test_defaults() {
    let src = "[characters]\nshalon = \"\\\\0\"\n";
    let config = parse(src);
    assert!(config.settings.shuffle_reset);
    assert_eq!(config.settings.encoding, "UTF-8");
}

#[test]

fn test_missing_settings() {
    let src = "[characters]\nshalon = \"\\\\0\"\nmurdock = \"\\\\1\"\n";
    let config = parse(src);
    assert_eq!(config.characters.len(),2);



}
#[test]
fn test_japanese_character_name_requires_quoted_key() {
    let src = "[characters]\n\"湊\" = \"\\\\0\"\n\"マードック\" = \"\\\\1\"\n";
    let config = parse(src);
    assert_eq!(config.characters.get("湊").map(|s| s.as_str()), Some("\\0"));
    assert_eq!(config.characters.get("マードック").map(|s| s.as_str()), Some("\\1"));
}

#[test]
fn test_backslash_escape_handled_by_toml_parser_alone() {
    // \\0 という記法がTOMLパーサー自身のエスケープ規則だけで
    // \0 (バックスラッシュ1文字+ゼロ) に変換されることの確認。
    // normalize_tag削除前後で結果が変わらないことを保証する。
    let src = "[characters]\n\"湊\" = \"\\\\0\"\n";
    let config = parse(src);
    let tag = config.characters.get("湊").expect("湊が見つからない");
    assert_eq!(tag.len(), 2, "バックスラッシュ1文字+ゼロの2バイトであるべき: {:?}", tag);
    assert_eq!(tag.as_bytes()[0], b'\\');
    assert_eq!(tag.as_bytes()[1], b'0');
}
#[test]
fn test_encoding_values() {
    assert!(parse("[settings]\nencoding = \"UTF-8\"\n").settings.response_is_utf8());
    assert!(parse("[settings]\nencoding = \"utf8\"\n").settings.response_is_utf8());
    assert!(!parse("[settings]\nencoding = \"Shift_JIS\"\n").settings.response_is_utf8());
    assert!(!parse("[settings]\nencoding = \"sjis\"\n").settings.response_is_utf8());
    // 書かなければUTF-8
    assert!(parse("[characters]\n").settings.response_is_utf8());
}

#[test]
fn test_unknown_encoding_is_load_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[settings]\nencoding = \"EUC-JP\"\n").unwrap();
    let err = Config::load(&path).unwrap_err();
    assert!(err.contains("EUC-JP"), "{}", err);
}
}
