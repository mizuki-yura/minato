# 設定（config.toml）

`config.toml` はゴーストフォルダに置く設定ファイルです。

## キャラクター設定

```toml
[characters]
湊 = "\\0"
助手 = "\\1"
```

キャラクター名とSAKURAスクリプトのタグを対応させます。
スクリプト内でキャラクター名を使うとここで設定したタグに変換されます。

```
OnBoot => {
    湊: こんにちは。    // \\0こんにちは。 に変換される
    助手: よろしく。    // \\1よろしく。 に変換される
}
```

## 動作設定

```toml
[settings]
shuffle_reset = true        # トークを一周したらリセットするか
talk_interval_secs = 300    # ランダムトークの基本間隔（秒）
talk_jitter_secs = 180      # ランダムトークの揺らぎ（秒）
debug_log = false           # trueにするとminato_load.logを生成する
```

### shuffle_reset

`true` のとき、`OnRandomTalk` の全候補を一周したら選択履歴をリセットします。
同じトークが偏って選ばれるのを防ぎます。

### talk_interval_secs / talk_jitter_secs

ランダムトークの発火間隔を設定します。
実際の間隔は `talk_interval_secs` から `talk_interval_secs + talk_jitter_secs` の間でランダムになります。

デフォルトは300〜480秒（5〜8分）です。

### debug_log

`true` にするとゴーストフォルダに `minato_load.log` が生成されます。
動作がおかしいときのデバッグに使います。
リリース時は `false` に戻してください。

## 最小構成

`[settings]` は省略できます。省略した場合はすべてデフォルト値が使われます。

```toml
[characters]
湊 = "\\0"
```