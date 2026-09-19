# 9. セーブデータの移行と型の違い

すでにユーザーがいるゴーストを湊に移すなら、里々の `satori_savedata.txt` を引き継ぐ必要があります。里々のセーブデータは湊では**読めません**。形式も、値の型も違います。

## 里々と湊のセーブデータ

| | 里々 | 湊 |
|---|---|---|
| ファイル | `satori_savedata.txt`（暗号化すると `.sat`） | `save.json` |
| 形式 | 1行に1変数。`＄変数名【タブ】値` | JSON |
| 保存される変数 | **すべての変数** | **`save` の下に置いたものだけ**（[5. 変数](variables.md)） |
| 値の型 | すべて文字列 | 数値・文字列・真偽値・配列・マップ・null |
| 文字コード | 既定 Shift_JIS（`is_utf8_savedata` で UTF-8 も可） | UTF-8（BOMなし） |
| 保存のタイミング | 終了時、`＄手動セーブ`、`＄自動セーブ間隔` | **終了時と再読み込み時だけ** |
| バックアップ | `satori_savebackup.txt` | 下記の `.bak` |

里々のセーブデータは、たとえばこのような形です。

```text
＊セーブデータ
＄好感度	１２
＄名前	ユーザー
＄体重	５７．８
＄前回起動	2026/09/18
＄喋り間隔	１８０秒
```

湊の `save.json` は、たとえばこのようになります。

```json
{
  "save": {
    "名前": "ユーザー",
    "好感度": 12.0
  },
  "system": {
    "debug_log": false,
    "talk_interval": 300.0,
    "talk_jitter": 180.0
  }
}
```

- `save` が、台本の `global save.…` の中身です。
- `system` は湊の設定で、`talk_interval`、`talk_jitter`、`debug_log` の3つだけが保存されます（下記）。
- **JSONの数値は、整数でも `3.0` のように保存されます。** 台本からは `3` として見えます。
- 保存のとき、キーは**辞書順**に並びます。再起動後、マップの要素の順序は辞書順になります。

## 移行の方法は2つ

| 方法 | 向く場面 |
|---|---|
| A. 湊の台本で読み込む | **配布済みのゴーストを更新するとき**。ユーザーの環境に残った `satori_savedata.txt` を、湊が最初に起動したときに自動で取り込む |
| B. 変換スクリプトで `save.json` を作る | 自分の環境のデータを移したいとき。ユーザーに渡す `save.json` の見本を作りたいとき |

## A. 湊の台本で読み込む

湊は、`ghost/master` の中のファイルを `file_read()` で読めます。里々の `satori_savedata.txt` は、**そのまま `ghost/master` に残っている**ので、湊が起動したときに読み込んで `save` に移せば、ユーザーは何もせずに引き継げます。

次の台本は、`satori_savedata.txt` を Shift_JIS として読み、`＄` で始まる行を `save` に入れます。全角の数字を半角にし、数値に見えるものは数値に直します。

<!--run OnBoot @save-->
```mnt
func 半角化(s) {
    let r = s
    let z = "０１２３４５６７８９"
    for (let i = 0; i < 10; i++) {
        r = replace(r, substr(z, i, 1), to_str(i))
    }
    r = replace(r, "．", ".")
    r = replace(r, "－", "-")
    return r
}

func 取り込み() {
    let text = file_read("ghost/master/satori_savedata.txt", "sjis")
    let n = 0
    foreach split(text, chr(10)) as i, line {
        let row = replace(line, chr(13), "")
        if (starts_with(row, "＄")) {
            let p = split(substr(row, 1), chr(9))
            if (len(p) >= 2) {
                let v = replace(p[1], "φ", "")
                let h = 半角化(v)
                if (regex_match(h, '^-?[0-9]+(\.[0-9]+)?$')) {
                    global save[p[0]] = to_num(h)
                } else {
                    global save[p[0]] = v
                }
                n += 1
            }
        }
    }
    return n
}

OnBoot => {
    // ここから3行は動作確認用。実際は、里々のセーブデータがすでに置かれている。
    let tab = chr(9)
    let nl = chr(10)
    file_write("ghost/master/satori_savedata.txt", "＊セーブデータ" + nl + "＄好感度" + tab + "１２" + nl + "＄名前" + tab + "ユーザー" + nl + "＄体重" + tab + "５７．８" + nl, "sjis")

    if (!save.移行済み) {
        let n = 取り込み()
        global save.移行済み = true
        湊: ${n}個の変数を取り込みました。
    }
}
```
```text
\03個の変数を取り込みました。\e
{"体重":57.8,"名前":"ユーザー","好感度":12.0,"移行済み":true}
```

- `取り込み` は、取り込んだ変数の数を返す関数です。`OnBoot` で、`save.移行済み` を目印にして、**最初の1回だけ**実行します。
- 正規表現の `$` は、**シングルクォート `'…'` の中に書きます**。ダブルクォート `"…"` の中の `$` は、`${` 以外だと構文エラーになります（[10. 文字コード・改行・エスケープ](encoding.md#-と--の扱い)）。
- `foreach 配列 as i, 要素` の形で、`i` が添字、2つ目が要素です。**変数を1つしか書かないと、それは添字になります。**
- `OnBoot` の最初の3行は、動作確認用です。実際のゴーストでは不要で、`if (!save.移行済み)` の部分だけを使います。
- `satori_savedata.txt` が UTF-8 なら、`file_read` の第2引数を `"utf8"` にします。
- **`satori_savedata.txt` が存在しないときは、`file_read` は `null` を返し、何も取り込まれません**（警告がログに残ります）。
- 里々の特殊変数（`喋り間隔` など）も取り込まれます。不要なものは、`save` に入れる前に条件で除外してください。

## B. 変換スクリプトで save.json を作る

Windows の PowerShell で動く変換スクリプトです。`satori_savedata.txt` を読み、`save.json` を書きます。

```powershell
# convert-satori-save.ps1
# Converts satori_savedata.txt to save.json (ASCII only on purpose).
param(
    [string]$In  = "satori_savedata.txt",
    [string]$Out = "save.json",
    [switch]$Utf8,
    [string[]]$Exclude = @()
)

# Resolve relative paths against the current PowerShell location (.NET does not follow cd).
$In  = [System.IO.Path]::Combine((Get-Location).Path, $In)
$Out = [System.IO.Path]::Combine((Get-Location).Path, $Out)

$dollar    = [string][char]0xFF04
$phi       = [string][char]0x03C6
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
if ($Utf8) { $enc = $utf8NoBom } else { $enc = [System.Text.Encoding]::GetEncoding(932) }

$toHalf = [System.Text.RegularExpressions.MatchEvaluator]{
    param($m)
    $c = [int][char]$m.Value
    if     ($c -eq 0xFF0E) { "." }
    elseif ($c -eq 0xFF0D) { "-" }
    else                   { [string][char]($c - 0xFEE0) }
}
$fullWidthNumberChars = "[" + [char]0xFF10 + "-" + [char]0xFF19 + [char]0xFF0E + [char]0xFF0D + "]"

$save = [ordered]@{}
foreach ($line in [System.IO.File]::ReadAllLines($In, $enc)) {
    if (-not $line.StartsWith($dollar)) { continue }
    $body = $line.Substring(1)
    $tab  = $body.IndexOf("`t")
    if ($tab -lt 0) { continue }
    $name = $body.Substring(0, $tab)
    if ($Exclude -contains $name) { continue }
    $value = $body.Substring($tab + 1).Replace($phi, "")
    $half  = [regex]::Replace($value, $fullWidthNumberChars, $toHalf)
    if ($half -match '^-?[0-9]+(\.[0-9]+)?$') {
        $save[$name] = [double]$half
    } else {
        $save[$name] = $value
    }
}

$json = @{ save = $save } | ConvertTo-Json -Depth 5
[System.IO.File]::WriteAllText($Out, $json, $utf8NoBom)
Write-Host ("converted: " + $save.Count)
```

使い方です。PowerShell を開き、`satori_savedata.txt` とスクリプトのあるフォルダに移動して、次のように実行します。

```
.\convert-satori-save.ps1 -In satori_savedata.txt -Out save.json -Exclude 喋り間隔,喋り間隔誤差
```

「スクリプトの実行が無効になっています」と表示されたときは、先に `Set-ExecutionPolicy -Scope Process Bypass` を実行してください（今開いているPowerShellだけで有効です）。

- 里々のセーブデータが UTF-8 なら、`-Utf8` を付けます。
- `-Exclude` に、移行しない変数名をカンマで並べます（里々の特殊変数など）。
- 「`＄`で始まり、タブで区切られた行」だけを読みます。見出しの `＊セーブデータ` やタブのない行は飛ばします。
- `φ`（里々が括弧を保護するために付ける印）は取り除きます。
- 全角数字を半角にして、`12`、`57.8`、`-3` のような**数値に見える値は数値**にします。それ以外（`１８０秒` や日付など）は文字列のままです。
- **BOMなしのUTF-8で書き出します。** BOM付きだと、湊が `save.json` を「壊れている」と判断して、空のセーブデータで起動します（元のファイルは `save.json.corrupt.bak` に残ります）。

> **スクリプトを自分で編集するときの注意** このスクリプトは意図的に**ASCIIの文字だけ**で書いてあります。Windows PowerShell 5.1 は、BOMなしのUTF-8で保存された `.ps1` を日本語Windowsの文字コード（Shift_JIS）として読むため、**日本語（コメントを含む）を書くと構文エラーになります**。日本語を書きたいときは、`.ps1` を「BOM付きのUTF-8」で保存してください。

上の里々のセーブデータ（`喋り間隔` は除外しない）から作られる `save.json` は、次のとおりです。全角の数字は数値になり、`φ` は消えています。

```json
{
    "save":  {
                 "好感度":  12,
                 "名前":  "ユーザー",
                 "体重":  57.8,
                 "前回起動":  "2026/09/18",
                 "喋り間隔":  "１８０秒"
             }
}
```

この `save.json` を湊のゴーストの `ghost/master` に置いて起動したとき、台本からは次のように見えます。

<!--run OnBoot-->
```json
{
    "save":  {
                 "好感度":  12,
                 "名前":  "ユーザー",
                 "体重":  57.8,
                 "前回起動":  "2026/09/18",
                 "喋り間隔":  "１８０秒"
             }
}
```
```mnt
OnBoot => {
    湊: ${save.好感度 + 1}|${save.好感度 >= 10}|${save.体重 * 2}
    湊: ${save.名前}さん、${save.喋り間隔}
    let c = regex_captures(save.前回起動, "(\d+)/(\d+)/(\d+)")
    湊: ${days_between(c[1], c[2], c[3], 2026, 9, 19)}日ぶりです。
}
```
```text
\013|true|115.6\nユーザーさん、１８０秒1日ぶりです。\e
```

## 型の違いを確認する

移行のときは、変数ごとに「湊ではどの型で持つか」を決めます。

| 里々での値 | そのまま移すと | 湊での持ち方 |
|---|---|---|
| 全角の数（`１２`） | 文字列 `"１２"`。**計算も比較もできない** | 数値 `12` に直す（上のスクリプトは自動） |
| 半角の数（`12`） | 文字列 `"12"`。`+ 1` は `121` になる | 数値 `12` に直す |
| フラグ（`0` / `1`、`有効` など） | `"0"` も**真**になる | `0`/`1` の数値、または `true`/`false` にする |
| 名前などの文字列 | そのまま文字列 | 文字列 |
| 日付（`2026/09/18`） | 文字列 | 文字列のまま、必要なときに `regex_captures` で分解する（上の例） |
| 番号付きの変数（`魔法少女０`、`魔法少女１`） | 別々の変数 | 配列 `[…]` にまとめる（自動変換はされない） |
| 里々の特殊変数（`喋り間隔` など） | 意味のない変数として残る | 移行しない（`-Exclude`） |

**移行後に、動作が変わりやすいのはフラグです。** 里々では `0` は偽でしたが、湊では文字列の `"0"` は真です。変換スクリプトは数値 `0`、`1` にするので問題ありませんが、`有効` や `無効` のような文字列のフラグは、台本側を `== "有効"` のように書き直すか、真偽値に変えてください。

## config.toml の間隔設定は save.json が勝つ

`talk_interval_secs`、`talk_jitter_secs`、`debug_log` は、**初回の起動時にだけ `config.toml` の値が使われ、そのあとは `save.json` の値が優先されます。** 起動して一度でも終了すると、`save.json` の `system` にこの3つが書き込まれるためです。

たとえば、最初に `talk_interval_secs` を 300 で起動して終了し、あとで `config.toml` を 60 に書き換えて起動すると、`system.talk_interval` は 300 のままです。`save.json` を消して起動し直すと、60 になります（実際にこの順序で確認しました）。

- ランダムトークの間隔を、配布済みのユーザーの環境で変えたいときは、`config.toml` を書き換えても効果がありません。**台本の中で `global system.talk_interval = 60` のように書き換えます。**
- `debug_log` も同じ仕組みです。ログを出したいときは、`save.json` の `system.debug_log` を `true` にするか、`save.json` を消してください。

<!--run OnClose OnBoot @reload OnClose-->
```mnt
OnBoot => {
    global system.talk_interval = 60
}

OnClose => {
    湊: ${system.talk_interval}
}
```
```text
\0300\e
(204)
(reload)
\060\e
```

台本から書き換えた値は、再起動しても残ります。

## 壊れたとき、消えたとき

| 状況 | 湊の動き |
|---|---|
| `save.json` が壊れている（JSONとして読めない） | 元のファイルを **`save.json.corrupt.bak`** に移し、空のセーブデータで起動する |
| 湊の実行中に内部エラーが起き、そのあとに保存する | 上書き前の `save.json` を **`save.json.panic.bak`** に残す。次の起動時に一度だけ警告が出る |
| 台本にエラーがあって読み込めなかった | **`save.json` は上書きしない**（セーブデータが空で潰れるのを防ぐ） |
| ゴーストが強制終了された | 保存されない。その起動中に覚えたことは消える |

里々の `satori_savebackup.txt`（1世代前のバックアップ）に当たる、通常の世代バックアップはありません。

## 配布するとき

`save.json` や `.bak` ファイルは、配布するファイル（NARなど）に含めません。里々の `satori_savedata.txt` と同じ扱いです。SSPでは、`developer_options.txt` に次のように書きます。

```
ghost/master/save.json,nonar,noupdate
```

## YAYAから来た方へ

YAYAのグローバル変数は `yaya_variable.cfg` に保存されます。このファイルの形式を読む変換ツールは、このガイドにはありません。移行するときは、変数の一覧を `ghost/master` に別の形式（`名前<タブ>値` の行など）で書き出す処理を、YAYA側に用意してください。それを上のAの台本のように、`file_read` で取り込むのが確実です。

次は[10. 文字コード・改行・エスケープ](encoding.md)に進んでください。
