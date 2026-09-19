# SAORI連携

SAORIは伺かのプラグイン規格です。外部のDLLを呼び出して、湊だけでは実現できない機能を追加できます。

## 基本

`saori(dll名, 引数0, 引数1, ...)` で呼び出します。
戻り値は配列です。

```
OnBoot => {
    let result = saori("hoge.dll", "引数1", "引数2")
    湊: 結果は${result[0]}です。
}
```

## DLLの配置

SAORIのDLLはゴーストフォルダに置きます。

```
ghost/master/
├── shiori.dll
├── config.toml
├── hoge.dll        ← SAORIのDLL
└── talks/
    └── main.mnt
```

## 戻り値

SAORIの戻り値は配列として返されます。
インデックス0が最初の戻り値です。

```
OnBoot => {
    let result = saori("hoge.dll", "引数")
    let value0 = result[0]
    let value1 = result[1]
}
```

## DLLの読み込みエラー

DLLが見つからない場合や読み込みに失敗した場合、パスが不正（絶対パスや `..` を含む）な場合は、
**空の文字列**が返されます（配列ではありません）。
`len(result)` は `0` になり、`result[0]` のように添字でアクセスすると `null` になって警告が出ます。

このときのエラー（`error` レベル）は、`debug_log` の設定に関係なく、SHIORI応答の `ErrorLevel` / `ErrorDescription` ヘッダに載ります。
`debug_log = true` にすると、`minato_load.log` にも詳細が記録されます。

戻り値が使えたかどうかは、`len(result)` で確かめてください。

## 注意

- 湊はSAORIに、引数を **UTF-8** で送ります（`Charset: UTF-8`）。応答もUTF-8として読みます。Shift_JISへの変換は行いません。
- ただし、DLLを読み込むときにSAORIへ渡すDLLのフォルダのパスは、Shift_JIS（表せない場合はUTF-8）です。
- リクエストの `Charset` ヘッダを見ずにShift_JISで処理する古いSAORIに日本語の引数を渡すと、文字化けや誤動作の原因になります。使うSAORIごとに確認してください。詳しくは[8. SAORI呼び出し](../migration/saori.md)を参照してください。
- DLLは初回呼び出し時に読み込まれ、ゴーストが終了（または再読み込み）されるまで保持されます。ただし、次の場合は例外です。
  - 同時に保持できるDLLは32個までです。超えると、最も長く使われていないものから解放されます。
  - 応答が5秒以内に返らなかったDLLは、そのゴーストを再読み込みするまで、二度と呼び出されません。
  - 1回のイベントの処理中に呼べる回数は、`saori()` と `get_property()` を合わせて10回までです。超えた呼び出しは実行されません。