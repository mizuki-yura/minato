
# func定義と呼び出し

## 基本

`func` キーワードで関数を定義します。

```
func greet(name) {
    return "こんにちは、" + name + "。"
}

OnBoot => {
    湊: ${greet("湊")}
}
```

## 引数なし

```
func random_greeting() {
    if (rand() % 2 == 0) {
        return "こんにちは。"
    }
    return "やあ。"
}

OnBoot => {
    湊: ${random_greeting()}
}
```

## 複数の引数

```
func add(a, b) {
    return a + b
}

OnBoot => {
    湊: ${add(3, 5)}です。
}
```

## グローバル変数の操作

関数の中から `global` でsaveを操作できます。

```
func increment(key) {
    global save[key] += 1
}

OnBoot => {
    increment("訪問回数")
    湊: ${save.訪問回数}回目です。
}
```

## 定義場所

関数はトークの外（トップレベル）に定義します。
`include` で別ファイルに分けることもできます。

## 再帰

再帰呼び出しは最大100回までです。