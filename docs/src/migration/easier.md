# 12. 湊だけで簡単になる書き方

ここまでは、壊れるものの話でした。この節では、逆に、里々では苦労した処理が湊で素直に書けるものを挙げます。移行の価値を判断する材料にしてください。

## 構造化されたデータ

里々では、`＄好感度＿湊`、`＄勝ち数`、`＄負け数` のように、変数名にキーを埋め込むしかありませんでした。湊では、配列とマップで**入れ子にして**持てます。途中のマップは自動で作られます。

<!--run OnBoot @save-->
```mnt
OnBoot => {
    global save.stats.win += 1
    global save.stats.lose += 2
    global save.fav.湊 = 10
    湊: ${save.stats.win}勝${save.stats.lose}敗
}
```
```text
\01勝2敗\e
{"fav":{"湊":10.0},"stats":{"lose":2.0,"win":1.0}}
```

## マップの繰り返し

`foreach マップ as キー, 値` で、キーと値を順に取り出せます。

<!--run OnBoot-->
```mnt
OnBoot => {
    let scores = {"りんご": 3, "みかん": 5}
    foreach scores as name, n {
        湊: ${name}は${n}点\n
    }
}
```
```text
\0りんごは3点\nみかんは5点\n\e
```

## 存在しないキーの安全な参照

`get(マップ, キー, 既定値)` で、キーがなくても警告を出さずに既定値が返ります。`??` は `null` のときだけ代替値を使います。

<!--run OnBoot-->
```mnt
OnBoot => {
    let m = {"a": 1}
    湊: ${get(m, "a", 0)}|${get(m, "b", 0)}|${m["b"] ?? "なし"}
}
```
```text
\01|0|なし\e
```

## 正規表現と日付の計算

里々では、SAORI（ssu）や自作の関数を組み合わせていた処理が、そのまま書けます。

<!--run OnBoot-->
```mnt
OnBoot => {
    let c = regex_captures("2026年1月2日", "(\d+)年(\d+)月(\d+)日")
    湊: ${c[1]}-${c[2]}-${c[3]}
    湊: ${days_between(2026, 1, 1, 2026, 1, 31)}日
}
```
```text
\02026-1-2\n30日\e
```

`regex_match`、`regex_find`、`regex_replace`、`regex_split` も使えます。`days_since(年, 月, 日)` は、指定した日から今日までの日数です。

## 関数と再帰

引数と戻り値のある関数が書けます。

<!--run OnBoot-->
```mnt
func 階乗(n) {
    if (n <= 1) {
        return 1
    }
    return n * 階乗(n - 1)
}

OnBoot => {
    湊: 5の階乗は${階乗(5)}
}
```
```text
\05の階乗は120\e
```

## かな順のソートと整形

`sort(配列, "kana")` は、ひらがな・カタカナ・濁点の違いを無視して、五十音順に並べます。`format` で桁を揃えられます。

<!--run OnBoot-->
```mnt
OnBoot => {
    湊: ${join(sort(["りんご", "ぶどう", "みかん"], "kana"), ",")}
    湊: ${format("%02d:%02d", 7, 5)}
}
```
```text
\0ぶどう,みかん,りんご\n07:05\e
```

## ファイルの読み書き

`ghost/master` の中のテキストファイルを、`file_read` / `file_write` / `file_append` で扱えます（`file_move` で移動もできます）。パスは、ゴーストのホームからの相対パスで、書き込めるのは `ghost/master` の中だけです。

<!--run OnBoot-->
```mnt
OnBoot => {
    let ok = file_write("ghost/master/memo.txt", "こんにちは")
    湊: ${ok}|${file_read("ghost/master/memo.txt")}
    湊: ${file_write("memo.txt", "x")}
}
```
```text
\0true|こんにちは\nfalse\e
```

- 文字コードの既定は UTF-8 です。Shift_JIS のファイルは、`file_read(パス, "sjis")` で読めます。
- `talks` の中、`config.toml`、`save.json`、`.dll`、`minato_` で始まるファイルには書き込めません。
- 1MBを超えるファイルは読めません。
- 書き込みは、途中で落ちても既存のファイルが壊れないように、一時ファイルを経由します（`file_append` を除く）。

## 事前の構文チェック

里々では、辞書のミスはゴーストを起動して初めて分かることが多くありました。湊には、SSPなしで動く構文チェッカー `minato_check` があります。

```
minato_check.exe "ゴーストのホームディレクトリ"
```

構文エラーは行番号つきで表示されます。さらに、次の静的チェックを行います。

| チェック | レベル |
|---|---|
| ループの外の `break` / `continue` | エラー（ゴーストは起動しない） |
| 存在しないトーク・関数を `call` している | 通知（notice） |

終了コードは、エラーがなければ `0`、あれば `1` です。ビルド・配布の手順に組み込めます。

## ログを残す

`log("メッセージ")` は、`config.toml`（と `save.json`）で `debug_log` が有効なときだけ、`minato_debug.log` に書き込みます。台本の途中経過を確認できます。

## 任意のトークが存在するかの確認

`talk_exists("名前")` で、トークまたは関数が定義されているかを確認できます。ゴーストの一部の機能を、ファイルの有無で切り替える設計にできます。

## 短絡評価

`&&`、`||`、`??` は、左辺で結果が決まると右辺を評価しません。`choose(条件, 値, 値)` も、選ばれなかった側の式は評価しません。副作用のある式（`saori()` など）を安全に書けます。

次は[13. 段階移行 vs 全置換](strategy.md)に進んでください。
