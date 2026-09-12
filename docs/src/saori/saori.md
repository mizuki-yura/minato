# SAORI連携

SAORIは伺かのプラグイン規格です。外部のDLLを呼び出して、湊だけでは実現できない機能を追加できます。

## 基本

`saori(dll名, 引数0, 引数1, ...)` で呼び出します。
戻り値は配列です。

```
OnBoot => {
    let result = saori("hoge.dll", "引数1", "引数2")
    湊: 結果は${result[0]}です。
}
```

## DLLの配置

SAORIのDLLはゴーストフォルダに置きます。

```
ghost/master/
├── shiori.dll
├── config.toml
├── hoge.dll        ← SAORIのDLL
└── talks/
    └── main.mnt
```

## 戻り値

SAORIの戻り値は配列として返されます。
インデックス0が最初の戻り値です。

```
OnBoot => {
    let result = saori("hoge.dll", "引数")
    let value0 = result[0]
    let value1 = result[1]
}
```

## DLLの読み込みエラー

DLLが見つからない場合や読み込みに失敗した場合は、
エラーログに記録されて空の配列が返されます。
`debug_log = true` にするとエラーの詳細を確認できます。

## 注意

- SAORIのDLLはShift_JISでやり取りする規格ですが、湊が自動的に変換します。
- DLLは初回呼び出し時に読み込まれ、ゴーストが終了するまで保持されます。