
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