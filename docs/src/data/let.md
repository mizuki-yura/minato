
# let（ローカル変数）

`let` はトークの中だけで使えるローカル変数です。
トークの実行が終わると消えます。

## 基本的な書き方

```
OnBoot => {
    let name = "湊"
    湊: 私の名前は${name}です。
}
```

## 値の種類

数値・文字列・bool・配列・マップが使えます。

```
OnBoot => {
    let count = 0
    let name = "湊"
    let flag = true
    let items = ["あ", "い", "う"]
    let data = {"key": "value"}
}
```

## 計算

```
OnBoot => {
    let a = 10
    let b = 3
    let sum = a + b
    湊: ${a}と${b}を足すと${sum}です。
}
```

## 再代入

`let` で宣言した変数は後から値を変えられます。

```
OnBoot => {
    let count = 0
    count += 1
    count += 1
    湊: ${count}回カウントしました。
}
```