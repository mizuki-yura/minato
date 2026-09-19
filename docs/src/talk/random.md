# ランダムトークの仕組み

## ランダムトークの発火

湊はSSPから `OnMinuteChange` イベントを受け取るたびに、
一定時間が経過していれば `OnRandomTalk` を実行します。
（`OnMinuteChange` は湊が内部で使うため、台本に `OnMinuteChange => { ... }` を書いても呼ばれません）

ただし、`OnMinuteChange` の `Reference3`（SSPがトークを再生できる状態かどうか。再生できるときは `1`）が `1` でないときは、一定時間が経過していても `OnRandomTalk` は実行されません。
たとえば、他のトークの再生中などでSSPがトークを受け付けられないときは、ランダムトークは発火しません。
次の `OnMinuteChange` で、あらためて判定されます。

時間の間隔は `config.toml` で設定できます。

```toml
[settings]
talk_interval_secs = 300   # 基本間隔（秒）
talk_jitter_secs = 180     # ゆらぎ（秒）
```

この例では300〜480秒のランダムな間隔でランダムトークが発火します。

初回起動後は `save.json` の `system.talk_interval` / `system.talk_jitter` が `config.toml` より優先されます。
詳しくは[設定（config.toml）](../config/config.md)を参照してください。

## 手動でのランダムトーク

SSPのメニューや `\a` タグからユーザーが手動でトークを要求すると
`OnAITalk` イベントが発生します。
湊は `OnAITalk` を `OnRandomTalk` として処理します。
手動のトークも `OnRandomTalk => { ... }` に書いてください（`OnAITalk => { ... }` は呼ばれません）。

## トーク選択の流れ

1. `OnRandomTalk` の全候補を取得する
2. 条件フィルタ（`if(...)`）を評価して候補を絞り込む
3. 候補からランダムにひとつ選ぶ（直前に選ばれたトークはなるべく避け、全候補が出そろうまで同じものは選ばない）
4. 選ばれたトークを実行する

## 直前のランダムトークの記録（save.last_talk）

`OnRandomTalk`（`OnAITalk` を含む）でトークが実行されるたびに、湊はその出力（末尾の `\e` を除いたさくらスクリプト）を `save.last_talk` に自動で保存します。
書いた覚えのない `last_talk` が `save.json` に入るのは、このためです。

```
OnRandomTalk => {
    湊: 前回のトークは「${save.last_talk}」でした。
}
```

不要であれば、無視して構いません。

## 仮想時刻

湊はSSPから現在時刻を取得して `now` 変数に格納します。
湊が起動してからSSPの時刻を取得できるまでは、日本標準時（UTC+9）の現在時刻が使われます。
`now` の詳細は[now・referenceの使い方](now.md)を参照してください