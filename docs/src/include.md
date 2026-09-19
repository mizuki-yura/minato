# include

`include` を使うと別の `.mnt` ファイルを読み込めます。
スクリプトが大きくなってきたときにファイルを分割するのに使います。

## 基本

```
include "random.mnt"
include "boot.mnt"
```

`main.mnt` の先頭に書くのが一般的です。
`main.mnt` は `talks/` フォルダの中にあるので、`talks/` は付けずに、`main.mnt` と同じフォルダからの相対パスで書きます。
`include "talks/random.mnt"` と書くと `talks/talks/random.mnt` を探してしまい、読み込みエラーになります。

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

- パスは、`include` を書いたファイルがあるフォルダからの相対パスです。`main.mnt` から書くときは `talks/` フォルダが基準ですが、サブフォルダの中のファイルから `include` するときは、そのサブフォルダが基準になります（たとえば `talks/sub/a.mnt` の中の `include "b.mnt"` は `talks/sub/b.mnt` を読み込みます）
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