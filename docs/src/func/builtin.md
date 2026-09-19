# ビルトイン関数

## 数値・算術

| 関数 | 説明 | 例 |
|---|---|---|
| `floor(n)` | 切り捨て | `floor(3.7)` → `3` |
| `ceil(n)` | 切り上げ | `ceil(3.2)` → `4` |
| `round(n)` | 四捨五入 | `round(3.5)` → `4` |
| `trunc(n)` | 小数部を切り捨て | `trunc(3.9)` → `3` |
| `abs(n)` | 絶対値 | `abs(-5)` → `5` |
| `min(a, b)` | 小さい方 | `min(3, 5)` → `3` |
| `max(a, b)` | 大きい方 | `max(3, 5)` → `5` |
| `clamp(n, lo, hi)` | 範囲内に収める | `clamp(15, 0, 10)` → `10` |
| `sqrt(n)` | 平方根 | `sqrt(4)` → `2` |
| `rand()` | ランダムな整数 | `rand() % 6` → `0`〜`5` |
| `PI()` | 円周率（括弧が必要。`PI` だけでは値になりません） | `PI()` → `3.14159...` |
| `to_rad(deg)` | 度をラジアンに変換 | `to_rad(180)` → `3.14159...` |
| `to_deg(rad)` | ラジアンを度に変換 | `to_deg(PI())` → `180` |
| `to_hex(n)` | 16進数文字列に変換 | `to_hex(255)` → `"ff"` |
| `to_hex(n, digits)` | 桁数指定で16進数に変換 | `to_hex(255, 4)` → `"00ff"` |

### 三角関数

| 関数 | 説明 |
|---|---|
| `sin(n)` | サイン（ラジアン） |
| `cos(n)` | コサイン（ラジアン） |
| `tan(n)` | タンジェント（ラジアン） |
| `asin(n)` | アークサイン |
| `acos(n)` | アークコサイン |
| `atan2(y, x)` | アークタンジェント |
## 文字列

| 関数 | 説明 | 例 |
|---|---|---|
| `len(s)` | 文字数 | `len("こんにちは")` → `5` |
| `contains(s, sub)` | 部分文字列を含むか | `contains("abcde", "bc")` → `true` |
| `starts_with(s, p)` | 文字列で始まるか | `starts_with("abc", "ab")` → `true` |
| `ends_with(s, p)` | 文字列で終わるか | `ends_with("abc", "bc")` → `true` |
| `replace(s, from, to)` | 文字列を置換 | `replace("abc", "b", "X")` → `"aXc"` |
| `split(s, sep)` | 文字列を分割して配列に | `split("a,b,c", ",")` → `["a","b","c"]` |
| `join(arr, sep)` | 配列を結合して文字列に | `join(["a","b","c"], ",")` → `"a,b,c"` |
| `trim(s)` | 前後の空白を除去 | `trim("  abc  ")` → `"abc"` |
| `substr(s, start, count)` | 部分文字列を取得 | `substr("abcde", 1, 3)` → `"bcd"` |
| `index_of(s, sub)` | 部分文字列の位置 | `index_of("abcde", "bc")` → `1` |
| `count(s, sub)` | 部分文字列の出現回数 | `count("pineapple", "p")` → `3` |
| `to_lower(s)` | 小文字に変換 | `to_lower("Pineapple")` → `"pineapple"` |
| `to_upper(s)` | 大文字に変換 | `to_upper("Pineapple")` → `"PINEAPPLE"` |
| `chr(n)` | 文字コードから文字に変換 | `chr(65)` → `"A"` |
| `to_str(n)` | 数値を文字列に変換 | `to_str(123)` → `"123"` |
| `to_num(s)` | 文字列を数値に変換 | `to_num("123")` → `123` |
| `format(fmt,…)` | 書式付き文字列に変換 | [詳細](format.md) |

`count` `contains` `starts_with` `ends_with` は大文字・小文字を区別します。
大文字小文字を無視して比較したい場合は `to_lower` で統一してから使ってください。

```
// 大文字小文字を無視してカウントする例
let n = count(to_lower("Pineapple"), "p")
// n == 3
```
## 正規表現

| 関数 | 説明 |
|---|---|
| `regex_match(s, pat)` | パターンに一致するか |
| `regex_find(s, pat)` | 最初に一致した文字列を返す |
| `regex_captures(s, pat)` | キャプチャグループを配列で返す |
| `regex_replace(s, pat, rep)` | パターンに一致した部分を置換 |
| `regex_split(s, pat)` | パターンで分割して配列に |

```
OnBoot => {
    let s = "2026年1月1日"
    let caps = regex_captures(s, "(\d+)年(\d+)月(\d+)日")
    湊: ${caps[1]}年${caps[2]}月${caps[3]}日ですね。
}
```

湊の文字列リテラルの中では `\` はそのまま1文字として扱われます。
正規表現の `\d` などは `\\d` ではなく `\d` と書いてください
（`\\d` と書くと `\` が2文字残り、意図したパターンに一致しません）。

## 配列

| 関数 | 説明 | 例 |
|---|---|---|
| `len(arr)` | 要素数 | `len([1,2,3])` → `3` |
| `first(arr)` | 最初の要素 | `first([1,2,3])` → `1` |
| `last(arr)` | 最後の要素 | `last([1,2,3])` → `3` |
| `push(arr, v)` | 末尾に追加した新しい配列を返す | `push([1,2], 3)` → `[1,2,3]` |
| `pop(arr)` | 末尾を除いた新しい配列を返す | `pop([1,2,3])` → `[1,2]` |
| `slice(arr, start, end)` | 部分配列を返す | `slice([1,2,3,4], 1, 3)` → `[2,3]` |
| `index_of(arr, v)` | 要素の位置 | `index_of([1,2,3], 2)` → `1` |
| `count(arr, v)` | 要素の出現回数 | `count([1,2,1], 1)` → `2` |
| `sort(arr)` | 昇順にソート | |
| `sort(arr, "desc")` | 降順にソート | |
| `sort(arr, "kana")` | かな順にソート | |
| `reverse(arr)` | 逆順にした新しい配列を返す | |
| `unique(arr)` | 重複を除いた新しい配列を返す | |

`push` `pop` `reverse` `unique` は元の配列を変更しません。新しい配列を返します。

```
OnBoot => {
    global save.履歴 ?= []
    global save.履歴 = push(save.履歴, "起動")
}
```

## マップ

| 関数 | 説明 | 例 |
|---|---|---|
| `has_key(map, key)` | キーが存在するか | `has_key(data, "name")` → `true` |
| `keys(map)` | キーの配列を返す | |
| `values(map)` | 値の配列を返す | |
| `delete(map, key)` | キーを除いた新しいマップを返す | |
| `len(map)` | キーの数 | |
| `get(map, key, default)` | キーの値を返す。なければ `default`（省略時は `null`） | `get(data, "age", 0)` |

`get` は配列にも使えます（`get(arr, 添字, default)`）。範囲外の添字や負の添字でも警告を出さず、`default` を返します。
`data["age"]` のように直接参照すると、存在しないキーや範囲外の添字では通知や警告が出ます。

## 日付・時刻

| 関数 | 説明 |
|---|---|
| `days_since(year, month, day)` | 指定した日から今日までの日数 |
| `days_between(y1, m1, d1, y2, m2, d2)` | 1つ目の日から2つ目の日までの日数（2つ目のほうが後なら正の数） |

存在しない日付（`2026, 2, 30` など）を渡すと、どちらも `null` を返します。

```
OnBoot => {
    let days = days_since(2026, 1, 1)
    湊: 2026年1月1日から${days}日経ちました。
}
```

## 制御・ユーティリティ

| 関数 | 説明 | 例 |
|---|---|---|
| `choose(cond, a, b)` | condが真なら`a`、偽なら`b`（選ばれなかった側の式は評価されません） | `choose(flag, "はい", "いいえ")` |
| `is_null(v)` | `v` が `null` か | `is_null(save.名前)` → `true` |
| `talk_exists(name)` | その名前のトークまたは関数が定義されているか | `talk_exists("挨拶")` → `true` |
| `log(v, ...)` | 値をログに書き出す（戻り値は `null`） | `log("起動しました")` |

`log` は、`debug_log` が有効なときだけ、ゴーストフォルダの `minato_debug.log` に `[SCRIPT] メッセージ` の形で書き込みます。
複数の値を渡すと `, ` でつないで1行にします。無効なときは何もしません。

## システム連携

| 関数 | 説明 |
|---|---|
| `get_property(name)` | SSPのプロパティを取得する |
| `saori(dll, arg0, arg1, ...)` | SAORIを呼び出す |

`saori` の詳細は[SAORI連携](../saori/saori.md)を参照してください。

## ファイル操作

| 関数 | 説明 | 戻り値 |
|---|---|---|
| `file_read(path, enc)` | テキストファイルを読み込む | 内容の文字列。失敗したら `null` |
| `file_write(path, text, enc)` | ファイルに書き込む（既存の内容は置き換え） | 成功したら `true`、失敗したら `false` |
| `file_append(path, text, enc)` | ファイルの末尾に追記する（なければ作る） | 成功したら `true`、失敗したら `false` |
| `file_move(from, to, overwrite)` | ファイルを移動（名前変更）する | 成功したら `true`、失敗したら `false` |

```
OnBoot => {
    let ok = file_write("ghost/master/memo.txt", "こんにちは")
    湊: ${ok}|${file_read("ghost/master/memo.txt")}
}
```

- `path` は、ゴーストのホーム（`ghost` フォルダの1つ上）からの相対パスで書きます。`/` でも `\` でも構いません。ドライブ指定・絶対パス・`..` を含むパスは使えません。
- `enc`（文字コード）は省略でき、既定は UTF-8 です。Shift_JIS のファイルは `"sjis"` を指定します（`"utf8"` も指定できます）。
- `file_read` で読めるのは 1MB までです。それを超えるファイルや、UTF-8 として読めないファイルは `null` になります。
- 書き込みできるのは `ghost/master` の中だけです。次のものには書き込めません（`file_move` の移動元・移動先も同じです）。
  - `talks` フォルダの中（台本）
  - `config.toml`、`descript.txt`、`save.json` で始まるファイル
  - `.dll` ファイル
  - `minato_` で始まるファイル
- `file_move` は、移動先に同名のファイルがあると、既定では移動せず `false` を返します。上書きするには、第3引数に `true` を指定します。フォルダは移動できません。
- `file_write` は一時ファイルを経由して書き込むので、途中で落ちても既存のファイルが壊れません（`file_append` は直接追記します）。
- 失敗したときは、応答の `ErrorLevel` / `ErrorDescription` に警告が載ります。禁止されたパスを指定した場合は、警告ではなくエラーになります。