md# 湊ドキュメント

[はじめに](README.md)

- [湊とは](intro/what_is_minato.md)
- [インストールと最初のゴースト](intro/install.md)

- [基本の書き方](basic/index.md)
  - [ファイル構成](basic/files.md)
  - [トーク定義](basic/talk.md)
  - [セリフの書き方](basic/dialogue.md)
  - [コメント](basic/comment.md)

- [変数とデータ](data/index.md)
  - [let（ローカル変数）](data/let.md)
  - [global / save（永続化）](data/global.md)
  - [値の種類](data/types.md)
  - [文字列展開](data/interpolation.md)

- [制御構文](control/index.md)
  - [if / else](control/if.md)
  - [for / foreach](control/for.md)
  - [while](control/while.md)
  - [match](control/match.md)
  - [break / continue / return](control/flow.md)

- [関数](func/index.md)
  - [func定義と呼び出し](func/funcdef.md)
  - [call文](func/call.md)
  - [ビルトイン関数](func/builtin.md)
  - [format関数](func/format.md)
- [トーク制御](talk/index.md)
  - [重み（@）](talk/weight.md)
  - [条件フィルタ](talk/cond.md)
  - [ランダムトークの仕組み](talk/random.md)
  - [now・referenceの使い方](talk/now.md)

- [SAORI連携](saori/saori.md)

- [設定（config.toml）](config/config.md)

- [include](include.md)

- [エラーと対処](error/index.md)
  - [パースエラーの読み方](error/parse.md)
  - [よくあるミス](error/common.md)