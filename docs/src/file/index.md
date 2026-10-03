# ファイルとJSON

湊の台本から、ゴーストのフォルダにあるテキストファイルを読み書きできます。
JSONの関数と組み合わせると、辞書や設定を別ファイルで管理できます。

## できること・できないこと

ゴーストを壊さないように、ファイル操作には制限があります。最初にここを確認してください。

| できること | できないこと |
|---|---|
| テキストファイルの読み込み（1MBまで） | 1MBを超えるファイルや、文字コードが合わないファイルの読み込み |
| `ghost/master` の中への書き込み・追記 | `ghost/master` の外への書き込み |
| ファイルの移動・名前変更（`file_move`） | ファイルの**コピー**と**削除**（関数がありません） |
| UTF-8 と Shift_JIS の読み書き | フォルダの作成・移動 |
| JSONの読み込み・書き出し | 絶対パス・ドライブ指定・`..` を含むパスの指定 |

`ghost/master` の中でも、次のものには書き込めません（`file_move` の移動元・移動先も同じです）。

- `talks` フォルダの中（台本）
- `config.toml`、`descript.txt`、`save.json` で始まるファイル
- `.dll` ファイル
- `minato_` で始まるファイル

コピーしたいときは `file_read` で読んで `file_write` で書き、削除したいときは空の内容を書き込むか、別の名前に移動して使わないようにしてください。

## ファイルの読み書き

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

- `path` は、ゴーストのホーム（`ghost` フォルダの1つ上）からの相対パスで書きます。`/` でも `\` でも構いません。
- `enc`（文字コード）は省略でき、既定は UTF-8 です。Shift_JIS のファイルは `"sjis"` を指定します（`"utf8"` も指定できます）。
- `file_move` は、移動先に同名のファイルがあると、既定では移動せず `false` を返します。上書きするには、第3引数に `true` を指定します。
- `file_write` は一時ファイルを経由して書き込むので、途中で落ちても既存のファイルが壊れません（`file_append` は直接追記します）。

## JSON

| 関数 | 説明 | 戻り値 |
|---|---|---|
| `json_parse(s)` | JSONの文字列を値（マップ・配列など）に変換する | 変換した値。失敗したら `null` |
| `json_stringify(v, pretty)` | 値をJSONの文字列に変換する。`pretty` が `true` なら字下げ付き、省略すると1行 | JSONの文字列。失敗したら `null` |

- マップのキーは、JSONに書かれた順（台本で入れた順）のまま保たれます。
- 先頭にBOMが付いたJSONも読めます。
- JSONの `null` を正しく読んだ場合も `null` が返りますが、このときは警告は出ません。
- 数値は内部ではすべて小数として扱います。整数で表せる値は `100` のように整数で書き出します。ただし 9007199254740992（2の53乗）を超える整数は正確には扱えません。
- JSONで表せない数値（NaN・無限大）は `null` として書き出します。
- `json_stringify` は、入れ子が100段を超える値や、結果が1MB（`file_read` で読める上限）を超える値は変換せず、警告を出して `null` を返します。

## 例：JSONで辞書を管理する

リポジトリの `examples/json_dict` に、JSONファイルでゴーストの辞書を管理するサンプルがあります。ここではその要点を紹介します。

`ghost/master/dict.json` に「分類名 → 言葉の配列」を書いておきます。

```json
{
  "挨拶": ["やあ。", "こんにちは。"],
  "話題": ["今日はいい天気だね。", "お茶でも飲もうか。"]
}
```

読み込みでは、`file_read` と `json_parse` のどちらが失敗しても空の辞書にしておきます。こうすると、後の処理が `null` を相手に警告を出し続けることがありません。

```
func dict_load() {
    let text = file_read("ghost/master/dict.json")
    if (is_null(text)) {
        global dict = {}
        return false
    }
    let data = json_parse(text)
    if (is_null(data)) {
        global dict = {}
        return false
    }
    global dict = data
    return true
}
```

保存では、手で編集しやすいように字下げ付きで書き出します。

```
func dict_save() {
    let text = json_stringify(dict, true)
    if (is_null(text)) {
        return false
    }
    return file_write("ghost/master/dict.json", text)
}
```

使う側では、戻り値がセリフに出ないように変数で受けます。

```
OnBoot => {
    let loaded = dict_load()
    うきわ君: ${dict_word("挨拶")}
}
```

ランダムに1つ選ぶ `dict_word`、追加・削除の `dict_add` / `dict_remove` など、残りの関数はサンプルの `talks/dict.mnt` を見てください。

## 失敗したとき

読み書きに失敗しても、台本は止まらずに `null` や `false` が返ります。原因の調べ方は[よくあるミス](../error/common.md#ファイルやjsonが読み込めない)を参照してください。
