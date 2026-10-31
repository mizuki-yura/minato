# 構文チェッカー

SSPにゴーストを読み込ませなくても、手元でスクリプトの誤りを確かめられるツールがあります。
構文エラーと静的解析の結果（未定義の `call`、ループの外の `break` など）を一覧で表示します。

チェックするだけなので、`save.json` を読み書きしたり、SAORIを読み込んだりはしません。

ツールは2種類あります。

| ツール | 向いている人 |
|---|---|
| `minato_check_gui.exe`（GUI版） | ダブルクリックで使いたい人 |
| `minato_check.exe`（CLI版） | コマンドラインやスクリプトから使いたい人 |

どちらも同じチェックを行います。

## GUI版（minato_check_gui）

1. `minato_check_gui.exe` をダブルクリックして起動します。
2. ゴーストのフォルダか、`talks` フォルダの中の `.mnt` ファイルを窓にドラッグ＆ドロップします。
   ドロップできないときは「フォルダを選ぶ」「ファイルを選ぶ」ボタンから選んでください。
3. エラー・警告・お知らせが色分けされて一覧で表示されます。

exeのアイコンにフォルダをドロップして起動することもできます。
前回選んだフォルダは記憶され、次に選択画面を開くとそこから始まります。

渡すフォルダは、ゴーストのフォルダ（`ghost/master` を含むフォルダ）でも、`ghost/master` そのものでもかまいません。
`talks/main.mnt` がある場所を自動で探します。

## CLI版（minato_check）

`talks/main.mnt` を含むフォルダ（ゴーストの `ghost/master`）を指定して実行します。

```
minato_check.exe "C:\ghost\mighost\ghost\master"
```

GUI版と違い、CLI版には `ghost/master` そのものを渡してください。

| オプション | 意味 |
|---|---|
| `--no-color` | 色付き表示をやめる（ログに保存するときなど） |
| `-h` / `--help` | 使い方を表示する |

診断は1件につき次のような形で表示されます。

```
[error] main.mntの4行目（OnBoot内）: メッセージ [ルールID] ヒント: …
```

静的チェックの結果には、末尾に `[unused-variable]` のようなルールIDが付きます（構文エラーには付きません）。

問題がなければ「構文・静的解析ともに問題ありませんでした」と表示されます。
診断は標準エラー出力に、問題なしのメッセージだけが標準出力に出ます。

### 終了コード

| 終了コード | 意味 |
|---|---|
| `0` | 問題なし、または warning / notice だけ |
| `1` | 構文エラーか、errorレベルの静的解析結果がある |

コミット前のチェックやCIに組み込むときは、この終了コードで判定してください。

### ドラッグ＆ドロップで使う

`minato_check.exe` と `tools\minato_check.bat` を同じフォルダに置くと、
`minato_check.bat` に `ghost/master` フォルダをドラッグ＆ドロップするだけでチェックできます。
結果を読めるように、終わったあと窓は閉じずに止まります。

### 自分でビルドする

Rustの環境があれば、ソースからビルドできます。

```
cargo build --release --features cli --bin minato_check
```

`--features cli` はチェッカー専用の依存を有効にするためのもので、本体DLLのビルドには必要ありません。

## 表示されるレベル

| レベル | 意味 |
|---|---|
| `error` | このままでは読み込めない。直す必要がある |
| `warning` | 動くが、意図どおりでない可能性が高い |
| `notice` | 動作には影響しない。念のための知らせ |

## 静的チェックの一覧

| ルールID | レベル | 見つけるもの |
|---|---|---|
| `loop-control-outside-loop` | error | ループの外の `break` / `continue`（[説明](common.md#ループの外でのbreak--continue)） |
| `unreachable-code` | warning | `return` / `break` / `continue` より後ろにあって、実行されない行（[説明](common.md#実行されない行)） |
| `undefined-call` | notice | トークにも関数にもない名前の呼び出し（[説明](common.md#callした名前のtypo)） |
| `free-variable` | notice | 関数・トークの中の、引数・`let`・`global` のどれでもない変数（[説明](common.md#呼び出し元の-let-に頼った関数トーク)） |
| `unused-variable` | notice | `let` で作ったのに一度も使っていない変数（[説明](common.md#使っていない変数)） |
| `write-only-variable` | notice | 値を入れているのに一度も読んでいない変数（[説明](common.md#使っていない変数)） |

`unused-variable` と `write-only-variable` は、名前を `_` で始めた変数には出ません。
関数の戻り値を捨てるために受ける変数などは、`let _ = dict_load()` のように書いてください。

それぞれのエラーの読み方は[パースエラーの読み方](parse.md)を、よくある原因は[よくあるミス](common.md)を見てください。
