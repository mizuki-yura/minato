# 3. イベント

里々のイベントには3種類あります。湊での扱いは、種類によって大きく違います。

| 種類 | 里々の例 | 湊 |
|---|---|---|
| ① ベースウェア（SSP）が送るイベント | `＊OnBoot`、`＊OnClose`、`＊OnMouseDoubleClick` | **ほぼそのまま**。`OnBoot =>` と書く |
| ② 里々が自動でやってくれること | ランダムトーク、`＄喋り間隔`、`OnGhostCalled` 未定義時の `OnBoot` 代用 | **一部だけ**。ランダムトークは対応、他は自前 |
| ③ 里々独自のイベント | なでられ、つつかれ、起動回数、`＊OnSatoriLoad` | **無い**。自分で作る |

## ① SSPのイベントはそのまま書ける

`＊OnBoot` は `OnBoot =>` に、`（Ｒ０）` は `reference["0"]` に置き換えます。

<!--run OnMouseDoubleClick[10|20|0|0|Head] OnMouseDoubleClick[10|20|0|0|Bust]-->
```mnt
OnMouseDoubleClick => {
    if (reference["4"] == "Head") {
        湊: 頭をつつかないでください。
    } else {
        湊: ${reference["4"]}ですね。
    }
}
```
```text
\0頭をつつかないでください。\e
\0Bustですね。\e
```

`OnMouseDoubleClick` の `Reference4` は当たり判定名です。どのイベントにどの Reference が付くかは、SSPのドキュメント（UKADOC）に従います。

### Reference は文字列

`reference` は、キーが文字列の番号（`"0"`, `"1"`, …）のマップで、**値はすべて文字列**です。`reference.0` とも書けます。

<!--run OnFoo[abc|def]-->
```mnt
OnFoo => {
    湊: ${reference.0}/${reference["1"]}
}
```
```text
\0abc/def\e
```

値が文字列なので、**数値のつもりで `+` を使うと連結になります。**

<!--run OnMouseClick[3]-->
```mnt
OnMouseClick => {
    湊: ${reference["0"] + 1}|${to_num(reference["0"]) + 1}
}
```
```text
\031|4\e
```

`reference["0"] + 1` は `"3" + 1` で `31` になります。里々の `（Ｒ０）＋１` は `4` でした。**数値として扱いたいときは `to_num()` を通してください。** 比較の `<` `>=` は自動で数値化されるので、そのまま使えます（詳しくは[5. 変数](variables.md#型と暗黙の型変換)）。

### 起動系のイベント

SSPは、`OnFirstBoot`、`OnGhostChanged`、`OnGhostCalled` に何も返さなかった（204）ときに、続けて `OnBoot` を送ります（SSPの仕様）。里々が持っていた「未定義なら `OnBoot` を代用する」動きは、SSP上ではそのまま期待できます。湊自身は代用処理をしません。

`OnFirstBoot` を使うなら、`OnFirstBoot =>` を定義します。初回に何か喋った場合、SSPは `OnBoot` を続けて送りません。初回のあとにも通常の起動トークを出したいときは、`OnFirstBoot` の中から `call 起動` のように呼びます。

## ② ランダムトーク

里々では、名前のない `＊` が「ランダムトーク」になり、間隔は `＄喋り間隔` で決まりました。湊では、**`OnRandomTalk` という名前のトーク**を書きます。同名を複数書けば、その中から選ばれます。

<!--any OnAITalk-->
```mnt
OnRandomTalk => {
    湊: 今日もいい天気ですね。
}

OnRandomTalk => {
    湊: 何か用ですか？
}
```
```text
\0今日もいい天気ですね。\e
\0何か用ですか？\e
```

### 発火のしくみ

- SSPが1分ごとに送る `OnMinuteChange` を、湊が数えます。前回から `talk_interval_secs`（既定300秒）＋ 0〜`talk_jitter_secs`（既定180秒）が経っていれば、`OnRandomTalk` を実行します。
- 起動直後にも、この間隔が適用されます。**起動してから最初のランダムトークまで、最短でも5分**かかります。
- 間隔は `config.toml` に書きます。ただし、**一度でも起動して `save.json` ができると、`config.toml` の値は無視されます**（[9. セーブデータ](savedata.md#configtoml-の間隔設定は-savejson-が勝つ)）。

### OnMinuteChange と OnAITalk は自分で定義できない

湊は、この2つのイベントを内部で処理します。**自分で `OnMinuteChange =>` や `OnAITalk =>` を書いても、呼ばれません。**

- `OnMinuteChange` は、上記のとおりランダムトークの発火に使われます。
- `OnAITalk`（SSPのメニューなどからユーザーが手動でランダムトークを求めたとき）は、`OnRandomTalk` に読み替えて実行されます。

<!--run OnAITalk OnMinuteChange[0|0|0|1]-->
```mnt
OnAITalk => {
    湊: OnAITalkは呼ばれない
}

OnRandomTalk => {
    湊: OnRandomTalkが呼ばれる
}

OnMinuteChange => {
    湊: OnMinuteChangeも呼ばれない
}
```
```text
\0OnRandomTalkが呼ばれる\e
\e
```

2行目の `\e` は、`OnMinuteChange` に対して湊が「何も喋らない」で返した応答です。`OnSecondChange` など、他のイベントは普通に自分で定義できます。

## ③ 里々独自のイベントは自分で作る

里々が用意してくれていた独自イベントは、湊にはありません。SSPのイベントを組み合わせて作ります。

| 里々 | 湊での作り方 |
|---|---|
| `＊0Headなでられ` などの撫で反応 | `OnMouseMove` の `reference["4"]`（当たり判定名）を数える |
| つつかれ（ダブルクリック反応） | `OnMouseDoubleClick` の `reference["3"]`（話者）と `reference["4"]` |
| 起動回数（情報取得変数） | `global save.起動回数 += 1` を `OnBoot` に書く |
| `＊OnSatoriLoad`（辞書ロード時） | `main.mnt` 直下に `global save.x ?= 0` を書く（ロード時に実行される） |
| `＄次のトーク`（トーク予約） | なし。さくらスクリプトの `\![raise,OnXxx]` などを使う |

### 撫で反応を作る

撫で反応は、`OnMouseMove` が続けて来た回数を数えて作ります。**セーブしたくないカウンタは `global work.…` に置きます**（`save` 以外は保存されません。[5. 変数](variables.md#裸の代入は起動中グローバルになる)）。

<!--run OnMouseMove[0|0|0|0|Head] OnMouseMove[0|0|0|0|Head] OnMouseMove[0|0|0|0|Head] OnMouseMove[0|0|0|0|Head] @save-->
```mnt
OnMouseMove => {
    if (reference["4"] == "Head") {
        global work.head += 1
        if (work.head >= 3) {
            global work.head = 0
            湊: くすぐったいです。
        }
    }
}
```
```text
(204)
(204)
\0くすぐったいです。\e
(204)
{}
```

3回目で反応し、カウンタが0に戻ります。最後の `{}` は、終了時に書き出される `save` の中身です。`work` は保存されていません。

## 湊が勝手にやること

里々が勝手にやっていたことがあるように、湊にも、書いていないのにやることがあります。挙動を比べるときに必要なので、まとめておきます。

| 何を | 内容 |
|---|---|
| 応答の末尾に `\e` を付ける | トークが出力した内容の最後に必ず付きます。出力が空だと、何も返しません（204） |
| `OnBoot`、`OnMinuteChange` の末尾に `\![get,property,OnGotVirtualTime,…]` を付ける | SSPの仮想時刻を取るためです。届いた時刻が `now` に入ります。`OnGotVirtualTime` は湊が内部で消費するので、自分では定義できません |
| `OnAITalk` を `OnRandomTalk` に読み替える | 上記 |
| `OnRandomTalk` の結果を `save.last_talk` に保存する | **書いた覚えのない変数が `save.json` に入ります。** 直前のランダムトークのさくらスクリプトが入っています。不要なら無視して構いません |
| `Status` ヘッダを `status` に展開する | `status.talking`, `status.choosing`, `status.minimizing`, `status.induction`, `status.passive`, `status.timecritical`, `status.nouserbreak`, `status.online`（真偽値）、`status.raw`（元の文字列） |
| `system` 変数を用意する | `system.talk_interval`, `system.talk_jitter`, `system.debug_log`, `system.ghost_dir`, `system.version` |

`save.last_talk` の実際の中身を確認しましょう。

<!--run OnAITalk @save-->
```mnt
OnRandomTalk => {
    湊: ふつうのトーク
}
```
```text
\0ふつうのトーク\e
{"last_talk":"\\0ふつうのトーク"}
```

`OnAITalk` の応答は `\0ふつうのトーク\e` で、そこから末尾の `\e` を除いた文字列が、`save.last_talk` に入ります（上のJSON表示では `\` が `\\` と書かれています）。

## OnTranslate を使うとき

里々の `replace.txt` の代わりに、SSPの `OnTranslate` イベント（さくらスクリプトを送信前に加工できる）を使うことができます。湊でも普通のイベントとして定義できます。

<!--run OnTranslate[\0あいう\e]-->
```mnt
OnTranslate => {
    return replace(reference["0"], "あ", "い")
}
```
```text
\0いいう\e\e
```

**湊は、応答の末尾に必ず `\e` を付けます。** `OnTranslate` に渡されたスクリプトは末尾に `\e` を含むので、返す文字列の末尾は `\e\e` になります。SSPがこれをどう扱うかは、SSP側での検証が必要です（この点は実機のSSPでは未確認です）。

次は[4. トーク](talks.md)に進んでください。
