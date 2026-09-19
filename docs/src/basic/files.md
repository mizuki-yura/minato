
# ファイル構成

湊のゴーストは以下のファイルで構成されます。

```
ghost/master/
├── shiori.dll       ← minato.dll をリネーム
├── config.toml      ← キャラクター設定・動作設定
├── save.json        ← 永続データ（自動生成）
└── talks/
    ├── main.mnt     ← メインスクリプト
    └── *.mnt        ← include で分割可能
```

## config.toml

キャラクターの設定と動作設定を書きます。詳細は[設定](../config/config.md)を参照してください。

## talks/

`.mnt` ファイルにトークとロジックを書きます。
`main.mnt` が必ず読み込まれます。ファイルが大きくなった場合は `include` で分割できます。

## save.json

`global save.*` で保存したデータが自動的にここに書き込まれます。
手動で編集する必要はありません。

## 自動的に作られるその他のファイル

湊は、ゴーストのフォルダ（`ghost/master/`）に、次のファイルを作ることがあります。

| ファイル | 内容 |
|---|---|
| `minato_load.log` | `debug_log = true` のときの動作ログ（追記されます） |
| `minato_request.log` | `debug_log = true` のときの、直近にSSPから届いたリクエストの内容（イベントごとに上書きされます） |
| `minato_debug.log` | `debug_log = true` のときに、台本の `log()` が書き出すログ（追記されます） |
| `save.json.corrupt.bak` | `save.json` が壊れていて読めなかったときに、元のファイルを退避したもの |
| `save.json.panic.bak` | 湊の内部エラーのあとに保存するとき、上書き前の `save.json` を退避したもの |
| `save.json.panic.bak.notified` | `save.json.panic.bak` について、警告をもう表示したことを示す空のファイル。これがあると、同じ退避について警告は繰り返し表示されません |
| `save.json.tmp` | `save.json` を保存している最中の一時ファイル。保存が終わると `save.json` に置き換わり、消えます |
| `(ファイル名).minato.tmp` | `file_write()` で書き込んでいる最中の一時ファイル。書き込みが終わると消えます |

これらは湊が管理するファイルなので、手動で編集する必要はありません。
`minato_` で始まるファイルは、台本の `file_write()` などからは書き込めません。
配布するときは、`save.json` や `.bak` などを含めないようにしてください。