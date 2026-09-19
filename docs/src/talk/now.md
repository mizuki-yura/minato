# now・referenceの使い方

## now

`now` は現在時刻を持つマップです。トークの中でいつでも参照できます。

| キー | 内容 | 例 |
|---|---|---|
| `now.年` | 年 | `2026` |
| `now.月` | 月 | `1`〜`12` |
| `now.日` | 日 | `1`〜`31` |
| `now.時` | 時 | `0`〜`23` |
| `now.分` | 分 | `0`〜`59` |
| `now.秒` | 秒 | `0`〜`59` |
| `now.曜日` | 曜日 | `0`=月〜`6`=日 |

## nowの使用例

```
OnRandomTalk => {
    if (now.時 >= 6 && now.時 < 12) {
        湊: おはようございます。
    } else if (now.時 >= 12 && now.時 < 18) {
        湊: こんにちは。
    } else {
        湊: こんばんは。
    }
}
```

## 曜日の判定

```
OnRandomTalk => {
    match now.曜日 {
        0 | 1 | 2 | 3 | 4 => {
            湊: 今日は平日ですね。
        }
        5 | 6 => {
            湊: 今日は休日ですね。
        }
    }
}
```

## reference

`reference` はSSPからイベントと一緒に渡される付加情報です。
マップ形式で、キーは番号の文字列です。

```
OnMouseDoubleClick => {
    湊: クリックされた場所はX=${reference["0"]}、Y=${reference["1"]}です。
}
```

どのイベントでどの `reference` が渡されるかはSSPのドキュメントを参照してください。

## status

`status` は、SSPがイベントと一緒に送る `Status` ヘッダの内容を持つマップです。
ゴーストの今の状態を、トークの中で判定できます。

| キー | 内容 |
|---|---|
| `status.talking` | 発話中か |
| `status.choosing` | 選択肢の表示中か |
| `status.minimizing` | 最小化中か |
| `status.induction` | 誘導中か |
| `status.passive` | パッシブモードか |
| `status.timecritical` | タイムクリティカルな状態か |
| `status.nouserbreak` | ユーザーによる中断ができない状態か |
| `status.online` | オンライン状態か |
| `status.raw` | `Status` ヘッダの元の文字列 |

`raw` 以外は `true` / `false` です。`Status` ヘッダにそのフラグが含まれていれば `true` になります。
ヘッダがないイベントでは、すべて `false` です（前のイベントの値は持ち越されません）。

```
OnRandomTalk => {
    if (status.minimizing) {
        // 最小化中は何も喋らない
        return
    }
    湊: こんにちは。
}
```

各フラグの意味の詳細は、SSPのドキュメントの `Status` ヘッダの項を参照してください。