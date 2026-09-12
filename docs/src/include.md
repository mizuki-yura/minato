# include

`include` を使うと別の `.mnt` ファイルを読み込めます。
スクリプトが大きくなってきたときにファイルを分割するのに使います。

## 基本

```
include "talks/random.mnt"
include "talks/boot.mnt"
```

`main.mnt` の先頭に書くのが一般的です。

## ファイル構成の例

```
ghost/master/
├── shiori.dll
├── config.toml
└── talks/
    ├── main.mnt        ← includeをまとめる
    ├── boot.mnt        ← 起動・終了系
    ├── random.mnt      ← ランダムトーク
    └── func.mnt        ← 関数定義
```

```
// main.mnt
include "boot.mnt"
include "random.mnt"
include "func.mnt"
```

## 注意

- パスは `talks/` フォルダからの相対パスです
- 同じファイルを複数回includeしても1回だけ読み込まれます
- includeは再帰的に使えます（includeしたファイルの中でincludeできます）

## トップレベルの定義

`include` したファイルでもトーク・関数・グローバル変数の定義がすべて使えます。

```
// func.mnt
func greet(name) {
    return "こんにちは、" + name + "。"
}

global save.バージョン = "1.0"
```