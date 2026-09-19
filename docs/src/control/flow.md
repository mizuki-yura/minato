# break / continue / return

## break

ループを途中で抜けます。`for` `foreach` `while` の中で使えます。

```
OnBoot => {
    let sum = 0
    for (let i = 0; i < 10; i++) {
        if (i == 5) {
            break
        }
        sum += i
    }
    湊: ${sum}で止まりました。
}
```

## continue

現在のループの残りの処理をスキップして次のループに進みます。

```
OnBoot => {
    let result = ""
    for (let i = 0; i < 5; i++) {
        if (i % 2 == 0) {
            continue
        }
        result += to_str(i)
    }
    湊: 奇数は${result}です。
}
```

## return

関数から値を返して終了します。`func` の中で使います。

```
func greet(name) {
    if (name == "") {
        return "名無しさん"
    }
    return name
}

OnBoot => {
    湊: こんにちは、${greet(save.名前)}。
}
```

値なしの `return` はそのまま関数を終了します。

```
func check() {
    if (!save.フラグ) {
        return
    }
    // フラグが真のとき（true、0以外の数値、空でない文字列など）だけここに来る
    global save.カウント += 1
}
```

## ループの外でのbreak / continue

ループの外で `break` や `continue` を使うとパースエラーになります。
静的チェックで検出されます。