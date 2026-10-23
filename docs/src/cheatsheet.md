# チートシート

湊の書き方を1ページにまとめた早見表です。くわしい説明は、各項目の末尾のリンク先を見てください。

## 湊の考え方

- 台本は `.mnt` ファイルに書きます。`talks/main.mnt` が必ず読み込まれ、`include` で分割できます（UTF-8・BOMなし）。
- 基本の単位は**トーク**です。`名前 => { … }` で定義し、イベント名もトーク名も同じ書き方です。
- 同じ名前のトークが複数あると、条件フィルタ `if(…)` で絞り込んだうえで、ランダムに1つ選ばれます。
- セリフとロジック（`let`・`if`・`for` など）は、同じブロックの中に混ぜて書けます。
- 変数は `let`（ブロック内だけ）、`global`（ゴースト全体）、`global save.…`（`save.json` に保存）の3段階です。**保存されるのは `save.` の下だけ**です。
- 記号はすべて半角です。全角の `（）` や `：` は、ただの文字として扱われます。
- 構文エラーが1つでもあると、ゴースト全体が読み込めません。[構文チェッカー](error/checker.md)で確かめてください。

<!--run OnBoot OnBoot-->
```mnt
OnBoot => {
    global save.訪問回数 ?= 0
    global save.訪問回数 += 1
    if (save.訪問回数 == 1) {
        湊: はじめまして。
    } else {
        湊: ${save.訪問回数}回目ですね。
        助手: よろしく。
    }
}
```
```text
\0はじめまして。\e
\02回目ですね。\1よろしく。\e
```

## トーク定義とセリフ

```mnt
OnRandomTalk => {
    湊: こんにちは。
    今日もいい天気ですね。
    [1]助手: そうだね。
}

OnRandomTalk => {
    湊: 何か用ですか？\w9
    \q[お茶にする,OnTea]
}
```

- `名前 => { … }` でトークを定義します。`{ }` は複数行で書きます（1行のブロックにセリフは書けません）。
- `キャラ名: セリフ` で話します。キャラ名は `config.toml` の `[characters]` に登録した名前です。
- 続けて書いた行は、`\n` でつながります。
- `[数字]` をキャラ名の前に書くと、サーフェスを切り替えます。
- `\w9`・`\q[…]` などのSAKURAスクリプトは、セリフの中にそのまま書きます。
- `//` と `/* … */` がコメントです。セリフの行末の `//` はコメントになりません。

→ [トーク定義](basic/talk.md)・[セリフの書き方](basic/dialogue.md)・[選択肢とウェイト](basic/choice.md)・[コメント](basic/comment.md)

## 変数とデータ

| 書き方 | 範囲 | 保存 |
|---|---|---|
| `let x = 1` | 宣言したブロックの中だけ | されない |
| `global work.x = 1` | ゴースト全体 | されない（終了で消える） |
| `global save.x = 1` | ゴースト全体 | `save.json` に保存される |
| `x = 1`（`let` なし） | 同名のローカル変数があれば更新、なければ起動中グローバル | されない |

代入演算子は `=` `+=` `-=` `*=` `/=` `%=` と、`null` のときだけ代入する `?=` です。

| 値の種類 | 例 |
|---|---|
| 数値 | `10`、`3.14`、`-5`（整数と小数の区別なし） |
| 文字列 | `"…"`（展開あり）、`'…'`（展開なし） |
| bool | `true`、`false` |
| 配列 | `["あ", "い"]`（添字は0始まり） |
| マップ | `{"name": "湊"}` |
| null | `null`（未定義の変数の値もこれ） |

| 文字列展開 | 意味 |
|---|---|
| `${name}` | 変数の値 |
| `${a + b}` | 式の結果 |
| `${len("abc")}` | 関数の戻り値 |
| `${save.名前 ?? "名無し"}` | `null` のときの代わりの値 |
| `${data["name"]}`、`${items[0]}` | マップ・配列の要素 |

- 未定義の変数は `null` です。`save.n == 0` は**偽**になるので、`?=` で初期化するか `(save.n ?? 0) == 0` と書きます。
- セリフの先頭で `体重は57.8kg` のように半角の `.` が続くと、変数参照とみなされて文が消えます。`${"体重は57.8kgです。"}` と書いてください。

→ [let](data/let.md)・[global / save](data/global.md)・[値の種類](data/types.md)・[文字列展開](data/interpolation.md)

## 制御構文

| やりたいこと | 書き方 |
|---|---|
| 条件で分ける | `if (条件) { … } else if (条件) { … } else { … }` |
| 回数を決めて繰り返す | `for (let i = 0; i < 5; i++) { … }` |
| 配列・マップを順に処理する | `foreach items as i, v { … }`（`i` は添字またはキー、`v` は値） |
| 条件が真のあいだ繰り返す | `while (条件) { … }` |
| 値で分ける | `match 値 { 0 => { … } 1 \| 2 => { … } _ => { … } }` |
| ループを抜ける | `break` |
| 次の周回へ進む | `continue` |
| 関数・トークを終える | `return`、`return 値` |

- 比較は `==` `!=` `<` `<=` `>` `>=`、論理は `&&` `||` `!` です。
- `for` の更新に書けるのは `i++` `i--` `i += n` `i -= n` だけです。
- `foreach items as i` と変数を1つだけ書くと、それは**添字**（マップならキー）です。
- ループは最大2000回で止まります。
- セリフの直後の行に `n += 1` などを書くと、セリフの続きとして出力されます。代入はセリフより前に書きます。

→ [if / else](control/if.md)・[for / foreach](control/for.md)・[while](control/while.md)・[match](control/match.md)・[break / continue / return](control/flow.md)

## 関数

<!--run OnBoot-->
```mnt
func greet(name) {
    return name + "さん、こんにちは。"
}

挨拶 => {
    湊: ${greet("ユーザー")}
}

OnBoot => {
    call 挨拶
    湊: 今日は${format("%02d", 7)}日です。
}
```
```text
\0ユーザーさん、こんにちは。今日は07日です。\e
```

| やりたいこと | 書き方 |
|---|---|
| 関数を定義する | `func 名前(引数, …) { … return 値 }`（ふつうはトップレベルに書く） |
| 戻り値を使う | `${名前(引数)}`、`let v = 名前(引数)` |
| トーク・関数のセリフを出す | `call 名前`（引数なし）、行頭に `名前(引数)` |
| 名前を実行時に決めて呼ぶ | `call 変数`（例: `call reference.0`） |
| 書式を指定して文字列にする | `format("%.1f", 57.8)` → `"57.8"`、`format("%03d", 7)` → `"007"` |

→ [func定義と呼び出し](func/funcdef.md)・[call文](func/call.md)・[format関数](func/format.md)

## トーク制御

```mnt
OnRandomTalk if(save.好感度 >= 50 && now.時 >= 18) => {
    湊: 夜ですね。
}

OnRandomTalk => {
    湊: こんにちは。
}

OnMouseDoubleClick => {
    湊: ${reference["4"]}をつつかれました。
}
```

- **条件フィルタ**: 名前のあとに `if(条件)` を付けると、条件を満たすときだけ候補に入ります。全候補が外れると何も出力されません（204）。
- **ランダムトーク**: `OnRandomTalk` に書きます。`config.toml` の間隔（既定300〜480秒）で発火します。手動のトーク（`OnAITalk`）も `OnRandomTalk` で処理されます。
- 候補は直前のトークをなるべく避け、全候補が出そろうまで同じものは選ばれません。
- **now**: `now.年` `now.月` `now.日` `now.時` `now.分` `now.秒` `now.曜日`。`now.曜日` は**0が月曜**、6が日曜です。
- **reference**: SSPから届いた付加情報です。`reference["0"]` または `reference.0` で読みます。値は文字列です。
- `OnAITalk`・`OnMinuteChange`・`OnGotVirtualTime` は湊が内部で処理するので、定義しても呼ばれません。

→ [条件フィルタ](talk/cond.md)・[ランダムトークの仕組み](talk/random.md)・[now・referenceの使い方](talk/now.md)

## 設定（config.toml）

```toml
[characters]
"湊" = "\\0"
"助手" = "\\1"

[settings]
talk_interval_secs = 300
talk_jitter_secs = 180
debug_log = false
auto_newline = true
```

| キー | 既定値 | 内容 |
|---|---|---|
| `[characters]` の各キー | — | キャラ名とスコープタグの対応。日本語のキーは引用符で囲み、値は `"\\0"` と書く |
| `talk_interval_secs` | `300` | ランダムトークの基本間隔（秒） |
| `talk_jitter_secs` | `180` | 間隔の揺らぎ（秒） |
| `debug_log` | `false` | `true` でログファイルを作る |
| `auto_newline` | `true` | 話者が戻ったとき、タグの直後に `\n` を入れる |

- `talk_interval_secs`・`talk_jitter_secs`・`debug_log` は、2回目以降の起動では `save.json` の `system.*` が優先されます。

→ [設定（config.toml）](config/config.md)

## よく使うビルトイン関数

| 用途 | 関数 |
|---|---|
| 乱数 | `rand()`、`rand(1, 6)`（両端を含む） |
| 数値 | `floor` `ceil` `round` `abs` `min` `max` `clamp` |
| 型の変換・判定 | `to_str` `to_num` `is_num` `is_str` `is_null` `type_of` |
| 文字列 | `len` `contains` `replace` `split` `join` `trim` `substr` `to_hankaku` `to_zenkaku` |
| 正規表現 | `regex_match` `regex_find` `regex_captures` `regex_replace`（`\d` は1つの `\` で書く） |
| 配列 | `len` `push` `pop` `first` `last` `slice` `index_of` `sort` `reverse` `unique` |
| マップ | `has_key` `keys` `values` `delete` `get(map, key, 既定値)` |
| 条件 | `choose(条件, a, b)`、`talk_exists("名前")` |
| 日付 | `days_since(年, 月, 日)`、`days_between(…)` |
| デバッグ | `log(値)`（`debug_log = true` のとき `minato_debug.log` へ） |
| 外部連携 | `get_property("…")`、`saori("x.dll", …)`（合わせて1イベント10回まで） |
| ファイル・JSON | `file_read` `file_write` `json_parse` `json_stringify` |

`push` や `sort` などは元の配列を変えず、新しい配列を返します。保存するときは `global save.履歴 = push(save.履歴, "起動")` のように代入し直します。

→ [ビルトイン関数](func/builtin.md)・[SAORI連携](saori/saori.md)・[ファイルとJSON](file/index.md)

## 里々・YAYAから来た方へ

特に気をつけたい違いです。

- **辞書は全部書き直し**です。`＊` `＠` の辞書は使えず、`名前 => { … }` の `.mnt` で書きます。
- **保存されるのは `global save.…` だけ**です。書き忘れてもエラーは出ず、終了すると消えます。
- **`+` は文字列があると連結**します（`"1" + 2` は `12`）。`reference` やSAORIの戻り値は文字列です。
- **`"0"` は真**です。偽になるのは `0`、空文字、`null`、空の配列・マップです。
- **曜日は0が月曜**です（YAYAは0が日曜）。

ほかの違いは[非互換一覧](migration/cheatsheet.md)に、移し方は[移行ガイド](migration/index.md)にまとめています。

## 関連リンク

- [インストールと最初のゴースト](intro/install.md)
- [ファイル構成](basic/files.md)
- [include](include.md)
- [エラーと対処](error/index.md)・[よくあるミス](error/common.md)
- [里々・YAYAから湊への移行](migration/index.md)
