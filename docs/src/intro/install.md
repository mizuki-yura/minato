# インストールと最初のゴースト

## 必要なもの

- SSP（最新版推奨）
- 湊のDLL（`minato.dll`）

## ファイル構成

ゴーストのフォルダに以下の構成を用意します。

```
ghost/master/
├── shiori.dll       ← minato.dll をリネーム
├── config.toml
└── talks/
    └── main.mnt
```

## config.toml の最小構成

```toml
[characters]
湊 = "\\0"
```

キャラクター名とSAKURAスクリプトのタグを対応させます。
サイドにキャラクターがいる場合は `\\1` も追加します。

```toml
[characters]
湊 = "\\0"
助手 = "\\1"
```

## main.mnt の最小構成

```
OnBoot => {
    湊: こんにちは。
}
```

これだけでSSPがゴーストを起動したとき「こんにちは。」と喋ります。

## 動作確認

SSPでゴーストをロードして起動セリフが出れば成功です。

うまく動かない場合は `config.toml` で `debug_log = true` に設定してください。
ゴーストフォルダに `minato_load.log` が生成され、エラーの詳細が記録されています。

```toml
[settings]
debug_log = true
```