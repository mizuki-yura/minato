# 湊 (Minato)

Rustで書かれた伺か/SSP用のSHIORIライブラリです。  
`.mnt`ファイルに書かれたスクリプトを読み込んでゴーストを動かします。

## 特徴

- セリフとロジックを同じブロックに書ける
- 永続データを配列・マップで構造化して保持できる
- 関数定義・match・foreachなどの複雑なロジックに対応
- パースエラーは日本語で行番号付きで表示
- SAORIで外部DLLと連携可能

## ドキュメント

https://mizuki-yura.github.io/minato/

## CLI構文チェッカー（minato_check）

SSP（伺か本体）に読み込ませなくても、talksスクリプトの構文エラーや
静的解析結果（未定義call、ループ外break等）を手元で確認できるCLIツールです。

```
cargo build --release --bin minato_check
target\i686-pc-windows-msvc\release\minato_check.exe "ゴーストのホームディレクトリ"
```

（ターゲットtripleは環境に合わせて読み替えてください）

`target\...\release\minato_check.exe` と `tools\minato_check.bat` を同じ
フォルダにコピーして配布すれば、Rust環境がない人でも `minato_check.bat` に
ゴーストのホームディレクトリ（`talks`フォルダを含む場所）をドラッグ&ドロップ
するだけで使えます。

- 終了コード `0`: 構文エラーなし（warning/noticeのみ、または問題なし）
- 終了コード `1`: 構文エラー、またはerrorレベルの静的解析結果あり

## ライセンス

MIT License（[LICENSE](./LICENSE)）  
@自由に使用・改変・再配布できます。著作権表示を残してください。