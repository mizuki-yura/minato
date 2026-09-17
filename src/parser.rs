// parser.rs - chumsky 0.10 対応版

use chumsky::prelude::*;
use chumsky::extra;
use chumsky::Parser;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
#[cfg(debug_assertions)] use std::io::Write;
// ── 型定義（変更なし）────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CmpOp { Eq, Ne, Lt, Le, Gt, Ge }

enum Either<L, R> { Left(L), Right(R) }

#[derive(Debug, Clone)]
pub enum BinOp { Add, Sub, Mul, Div,Mod}

#[derive(Debug, Clone)]
pub enum AssignOp { Set, Add, Sub, Mul, Div, Mod,SetIfNull }

#[derive(Debug, Clone)]
pub enum Expr {
    Str(String),
    Number(f64),
    Bool(bool),
    Var(Vec<String>),
    InterpolatedStr(Vec<StrPart>),
    Call(String, Vec<Expr>),
    Array(Vec<Expr>),
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    Index(Box<Expr>, Box<Expr>),
    Map(Vec<(Expr, Expr)>),
    NullCoalesce(Box<Expr>, Box<Expr>),
    Cmp(Box<Expr>, CmpOp, Box<Expr>),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone)]
pub enum PathSegment { Key(String), Index(Expr) }
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum StrPart { Lit(String), Var(Vec<String>), Expr(Expr) }

#[derive(Debug, Clone)]
pub struct Line {
    pub surface: Option<u32>,
    pub character: Option<String>,
    pub content: Vec<StrPart>,
}

/// Stmtに、由来する元ソースの行番号を添えたもの。
/// preprocessで結合された行の場合も、行番号はpreprocess前の元ファイル基準になる。
#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub node: T,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Dialogue(Line),
    Let(String, Expr),
    Global(Vec<PathSegment>, AssignOp, Expr),
    Assign(Vec<PathSegment>, AssignOp, Expr),
    If(Expr, Vec<Spanned<Stmt>>, Option<Vec<Spanned<Stmt>>>),
    For { init: Box<Stmt>, cond: Expr, step: Box<Stmt>, body: Vec<Spanned<Stmt>> },
    ForEach { collection: Expr, key: String, value: Option<String>, body: Vec<Spanned<Stmt>> },
    While(Expr, Vec<Spanned<Stmt>>),
    Call(Expr),
    FuncDef { name: String, params: Vec<String>, body: Vec<Spanned<Stmt>> },
    Return(Expr),
    Break,
    Continue,
    Match { expr: Expr, arms: Vec<MatchArm> },
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub patterns: Vec<MatchPattern>,
    pub body: Vec<Spanned<Stmt>>,
}

#[derive(Debug, Clone)]
pub enum MatchPattern {
    Value(Expr),
    Wildcard,
}

#[derive(Debug, Clone)]
pub struct Talk {
    pub event: String,
    pub cond: Option<Expr>,
    pub body: Vec<Spanned<Stmt>>,
}

// ── パーサー ─────────────────────────────────────────────

fn line_comment<'a>() -> impl Parser<'a, &'a str, (), extra::Err<Rich<'a, char>>> + Clone {
    just("//")
        .then(any().filter(|&c: &char| c != '\n').repeated())
        .ignored()
}

fn block_comment<'a>() -> impl Parser<'a, &'a str, (), extra::Err<Rich<'a, char>>> + Clone {
    just("/*")
        .then(just("*/").not().ignore_then(any()).repeated())
        .then(just("*/").labelled("「/*」コメントは「*/」で閉じてください"))
        .ignored()
}

fn ws<'a>() -> impl Parser<'a, &'a str, (), extra::Err<Rich<'a, char>>> + Clone {
    any().filter(|c: &char| *c == ' ' || *c == '\t')
        .ignored()
        .or(line_comment())
        .or(block_comment())
        .repeated()
        .ignored()
}

fn ws_nl<'a>() -> impl Parser<'a, &'a str, (), extra::Err<Rich<'a, char>>> + Clone {
    any().filter(|c: &char| c.is_whitespace())
        .ignored()
        .or(line_comment())
        .or(block_comment())
        .repeated()
        .ignored()
}

fn ident<'a>() -> impl Parser<'a, &'a str, String, extra::Err<Rich<'a, char>>> + Clone {
    any().filter(|c: &char| c.is_alphanumeric() || *c == '_' || (*c as u32) > 0x7F)
        .repeated()
        .at_least(1)
        .collect::<String>()
}
/// 識別子構成文字の判定（identやvar_pathの文字判定と揃える）
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || (c as u32) > 0x7F
}

/// 予約語を、識別子の一部としてではなく単独のトークンとして厳密にマッチさせる。
/// just("true") のような前方一致だけだと、"truename" のような識別子の先頭を
/// 誤ってキーワードとして消費してしまう（match文の '_' パターンが元々
/// この対策を持っていたので、それを他の予約語にも揃える）。
fn keyword<'a>(kw: &'static str) -> impl Parser<'a, &'a str, &'a str, extra::Err<Rich<'a, char>>> + Clone {
    just(kw)
        .then_ignore(any().filter(|&c: &char| is_ident_char(c)).not())
}

fn var_path<'a>() -> impl Parser<'a, &'a str, Vec<String>, extra::Err<Rich<'a, char>>> + Clone {
    ident()
        .separated_by(just('.'))
        .at_least(1)
        .collect::<Vec<_>>()
}

fn number<'a>() -> impl Parser<'a, &'a str, Expr, extra::Err<Rich<'a, char>>> + Clone {
    text::int(10)
        .then(just('.').then(text::digits(10)).or_not())
        .to_slice()
        .map(|s: &str| Expr::Number(s.parse().unwrap()))
}

fn str_lit<'a>() -> impl Parser<'a, &'a str, Expr, extra::Err<Rich<'a, char>>> + Clone {
    just('\'')
        .ignore_then(
            any().filter(|&c: &char| c != '\'')
                .repeated()
                .collect::<String>()
        )
        .then_ignore(just('\'').labelled("文字列（'...'）は「'」で閉じてください"))
        .map(Expr::Str)
}

enum AccessOp { Index(Expr), Field(String) }
// ──────────────────────────────────────────────────────────
// (A) expr() : cmp_expr に単独「=」検出を追加
// ──────────────────────────────────────────────────────────

fn expr<'a>() -> impl Parser<'a, &'a str, Expr, extra::Err<Rich<'a, char>>> + Clone {
    recursive(|expr| {
        // "..." 展開あり文字列
        let var_part_interp = just("${")
            .ignore_then(expr.clone())
            .then_ignore(just('}').labelled("変数展開「${」は「}」で閉じてください"))
            .map(StrPart::Expr);

        let interp_str = just('"')
            .ignore_then(
                just("\\\\").to(StrPart::Lit("\\\\".to_string()))
                    .or(var_part_interp)
                    .or(
                        just('\\')
                            .then(
                                any().filter(|&c: &char| c != '\r' && c != '\n' && c != '\\' && c != '"' && c != '$')
                                    .repeated()
                                    .at_least(1)
                                    .collect::<String>()
                            )
                            .map(|(bs, rest): (char, String)| {
                                StrPart::Lit(std::iter::once(bs).chain(rest.chars()).collect())
                            })
                    )
                    .or(
                        any().filter(|&c: &char| c != '\r' && c != '\n' && c != '$' && c != '\\' && c != '"')
                            .repeated()
                            .at_least(1)
                            .collect::<String>()
                            .map(StrPart::Lit)
                    )
                    .repeated()
                    .collect::<Vec<_>>()
            )
            .then_ignore(just('"').labelled("文字列（\"...\"）は「\"」で閉じてください"))
            .map(Expr::InterpolatedStr);

        let atom = {
            // !expr

            // func(args...)
            let call = ident()
                .then_ignore(just('('))
                .then(
                    expr.clone()
                        .separated_by(just(',').padded_by(ws()))
                        .allow_trailing()
                        .collect::<Vec<_>>()
                )
                .then_ignore(just(')').labelled("関数の引数リストは「)」で閉じてください"))
                .map(|(name, args)| Expr::Call(name, args));

       // [a, b, ...]
            let array = just('[')
                .ignore_then(ws_nl())
                .ignore_then(
                    expr.clone()
                        .separated_by(just(',').padded_by(ws_nl()))
                        .allow_trailing()
                        .collect::<Vec<_>>()
                )
                .then_ignore(ws_nl())
                .then_ignore(just(']').labelled("配列リテラルは「]」で閉じてください"))
                .map(Expr::Array);




                // マップリテラルのキー用パーサー。
// 裸の識別子（クォートなし）は「変数として評価」ではなく
// 「その名前自身をキー文字列として使う」と解釈する
// （m.a が変数aではなく文字列"a"として扱われるのと一貫性を取るため）。
// 動的なキーが必要な場合は [expr] の計算キー構文を使う。
let map_key = just('[')
    .ignore_then(ws())
    .ignore_then(expr.clone())
    .then_ignore(ws())
    .then_ignore(just(']').labelled("計算キー「[」は「]」で閉じてください"))
    .or(str_lit())
    .or(interp_str.clone())
    .or(number())
    .or(ident().map(Expr::Str));

let map_lit = just('{')
    .ignore_then(ws_nl())
    .ignore_then(
        map_key                          // ★ expr.clone() → map_key
            .then_ignore(ws_nl())
            .then_ignore(just(':'))
            .then_ignore(ws_nl())
            .then(expr.clone())           // 値側は変更なし
            .separated_by(just(',').padded_by(ws_nl()))
            .allow_trailing()
            .collect::<Vec<_>>()
    )
    .then_ignore(ws_nl())
    .then_ignore(just('}').labelled("マップリテラルは「}」で閉じてください"))
    .map(Expr::Map);
    
            let paren = just('(')
                .ignore_then(ws())
                .ignore_then(expr.clone())
                .then_ignore(ws())
                .then_ignore(just(')').labelled("「(」は「)」で閉じてください"));

            // 負の数値リテラル
            let neg = just('-')
                .then(text::int(10))
                .then(just('.').then(text::digits(10)).or_not())
                .to_slice()
                .map(|s: &str| Expr::Number(s.parse::<f64>().unwrap()));

             str_lit()
        .or(interp_str)
        .or(neg)
        .or(number())
        .or(keyword("true").to(Expr::Bool(true)))
        .or(keyword("false").to(Expr::Bool(false)))
        .or(map_lit)
        .or(array)
        .or(call)
        .or(var_path().map(Expr::Var))
        .or(paren)
        };

        // 添字・フィールドアクセス
        let indexed = atom.clone()
            .then(
                just('[')
                    .ignore_then(ws())
                    .ignore_then(expr.clone())
                    .then_ignore(ws())
                    .then_ignore(just(']').labelled("添字アクセスは「]」で閉じてください"))
                    .map(AccessOp::Index)
                .or(
                    just('.')
                        .ignore_then(ident())
                        .map(AccessOp::Field)
                )
                .repeated()
                .collect::<Vec<_>>()
            )
            .map(|(base, ops)| {
                ops.into_iter().fold(base, |acc, op| match op {
                    AccessOp::Index(idx) => Expr::Index(Box::new(acc), Box::new(idx)),
                    AccessOp::Field(key) => Expr::Index(Box::new(acc), Box::new(Expr::Str(key))),
                })
            });

  let unary = just('!')
    .then_ignore(ws())
    .repeated()
    .count()
    .then(indexed.clone())
    .map(|(n, e)| (0..n).fold(e, |acc, _| Expr::Not(Box::new(acc))));

// * /
let mul_div = unary.clone()
    .then(
        ws().ignore_then(
            just('*').to(BinOp::Mul)
                .or(just('/').then_ignore(just('=').not()).to(BinOp::Div))
                .or(just('%').then_ignore(just('=').not()).to(BinOp::Mod))
        )
        .then_ignore(ws())
        .then(unary.clone())   // ← indexed から変更
        .repeated()
        .collect::<Vec<_>>()
    )
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |acc, (op, rhs)| {
                    Expr::BinOp(Box::new(acc), op, Box::new(rhs))
                })
            });

        // + -
        let add_sub = mul_div.clone()
            .then(
                ws().ignore_then(
                    just('+').to(BinOp::Add)
                        .or(just('-').to(BinOp::Sub))
                )
                .then_ignore(ws())
                .then(mul_div.clone())
                .repeated()
                .collect::<Vec<_>>()
            )
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |acc, (op, rhs)| {
                    Expr::BinOp(Box::new(acc), op, Box::new(rhs))
                })
            });

        // 比較演算子
        let cmp_expr = add_sub.clone()
            .then(
                ws().ignore_then(
                    just("==").to(CmpOp::Eq)
                        .or(just("!=").to(CmpOp::Ne))
                        .or(just("<=").to(CmpOp::Le))
                        .or(just(">=").to(CmpOp::Ge))
                        .or(just('<').to(CmpOp::Lt))
                        .or(just('>').to(CmpOp::Gt))
                        // ★追加: 単独「=」（==ではない）を比較ミスとして検出
                        //   ==として回復しつつ、=の位置にエラーをemitする
                        .or(
                            just('=')
                                .then(just('=').not())
                                .validate(|_, e, emitter| {
                                    emitter.emit(Rich::custom(
                                        e.span(),
                                        "条件式で「=」が使われています。比較なら「==」を使ってください".to_string(),
                                    ));
                                    CmpOp::Eq
                                })
                        )
                )
                .then_ignore(ws())
                .then(add_sub.clone())
                .or_not()
            )
            .map(|(lhs, rhs)| match rhs {
                Some((op, r)) => Expr::Cmp(Box::new(lhs), op, Box::new(r)),
                None => lhs,
            });

        // && ||
        let and_or = cmp_expr.clone()
            .then(
                ws().ignore_then(
                    just("&&").to(true)
                        .or(just("||").to(false))
                )
                .then_ignore(ws())
                .then(cmp_expr.clone())
                .repeated()
                .collect::<Vec<_>>()
            )
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |acc, (is_and, next)| {
                    if is_and {
                        Expr::And(Box::new(acc), Box::new(next))
                    } else {
                        Expr::Or(Box::new(acc), Box::new(next))
                    }
                })
            });

        // ??
        and_or.clone()
            .then(
                ws().ignore_then(just("??"))
                    .then_ignore(ws())
                    .then(and_or.clone())
                    .or_not()
            )
            .map(|(lhs, rhs)| match rhs {
                Some((_, r)) => Expr::NullCoalesce(Box::new(lhs), Box::new(r)),
                None => lhs,
            })
    })
}
// ── interpolated_str ─────────────────────────────────────

fn interpolated_str<'a>(
    ex: impl Parser<'a, &'a str, Expr, extra::Err<Rich<'a, char>>> + Clone + 'a,
) -> impl Parser<'a, &'a str, Vec<StrPart>, extra::Err<Rich<'a, char>>> + Clone {

    let var_part = just("${")
        .ignore_then(ex.clone())
        .then_ignore(just('}').labelled("変数展開「${」は「}」で閉じてください"))
        .map(StrPart::Expr);

    let bare_var = ident()
        .then_ignore(just('.'))
        .then(ident())
        .then(
            just('.')
                .ignore_then(ident())
                .map(|k| Either::Left(k))
            .or(
                just('[')
                    .ignore_then(ws())
                    .ignore_then(ex.clone())
                    .then_ignore(ws())
                    .then_ignore(just(']').labelled("文字列中の添字アクセスは「]」で閉じてください"))
                    .map(|e| Either::Right(e))
            )
            .repeated()
            .collect::<Vec<_>>()
        )
        .map(|((head, first_field), rest)| {
            let base = Expr::Var(vec![head]);
            let acc = Expr::Index(Box::new(base), Box::new(Expr::Str(first_field)));
            rest.into_iter().fold(acc, |acc, op| match op {
                Either::Left(k)  => Expr::Index(Box::new(acc), Box::new(Expr::Str(k))),
                Either::Right(e) => Expr::Index(Box::new(acc), Box::new(e)),
            })
        })
        .map(StrPart::Expr);

    let sakura_tag = just('\\')
        .then(
            any().filter(|&c: &char| c != '\r' && c != '\n' && c != '\\' && c != '$')
                .repeated()
                .at_least(1)
                .collect::<String>()
        )
        .map(|(bs, rest): (char, String)| {
            let tag: String = std::iter::once(bs).chain(rest.chars()).collect();
            StrPart::Lit(tag)
        });

    let lit_part = any().filter(|&c: &char| c != '\r' && c != '\n' && c != '$' && c != '\\')
        .repeated()
        .at_least(1)
        .collect::<String>()
        .map(StrPart::Lit);

    var_part.or(bare_var).or(sakura_tag).or(lit_part)
        .repeated()
        .collect::<Vec<_>>()
}

// ── surface ──────────────────────────────────────────────

fn surface<'a>() -> impl Parser<'a, &'a str, u32, extra::Err<Rich<'a, char>>> + Clone {
    just('[')
        .ignore_then(
            text::int(10).validate(|s: &str, e, emitter| {
                s.parse::<u32>().unwrap_or_else(|_| {
                    emitter.emit(Rich::custom(
                        e.span(),
                        "サーフェス番号が大きすぎます（0〜4294967295の範囲で指定してください）".to_string(),
                    ));
                    u32::MAX
                })
            })
        )
        .then_ignore(just(']').labelled("サーフェス番号は「]」で閉じてください"))
}

// ── dialogue ─────────────────────────────────────────────

fn dialogue<'a>() -> impl Parser<'a, &'a str, Stmt, extra::Err<Rich<'a, char>>> + Clone {
    // キャラ指定あり: [0]湊: セリフ
    let with_chara = surface()
        .or_not()
        .then(ident())
        .then_ignore(ws())
        .then_ignore(just(':').labelled("キャラ名の後に「:」が必要です"))
        .then_ignore(ws())
        .then(interpolated_str(expr()))
        .then_ignore(ws())
        .then_ignore(
            just('\r').or_not().ignore_then(just('\n')).or_not()
        )
        .map(|((surf, chara), content)| {
            Stmt::Dialogue(Line { surface: surf, character: Some(chara), content })
        });

 
// キャラ指定なし: \- や \e など（バックスラッシュで始まる行のみ）
let without_chara = surface()
    .or_not()
    .then(
        // バックスラッシュで始まることを要求
        just('\\')
            .ignore_then(
                any().filter(|&c: &char| c != '\r' && c != '\n')
                    .repeated()
                    .collect::<String>()
            )
            .map(|rest| vec![StrPart::Lit(format!("\\{}", rest))])
    )
    .then_ignore(ws())
    .then_ignore(
        just('\r').or_not().ignore_then(just('\n')).or_not()
    )
    .map(|(surf, content)| {
        Stmt::Dialogue(Line { surface: surf, character: None, content })
    });
    with_chara.or(without_chara)
}



// ── path_segments / assign_op（global・assignで共用）─────

fn path_segments<'a>() -> impl Parser<'a, &'a str, Vec<PathSegment>, extra::Err<Rich<'a, char>>> + Clone {
    let index_key = just('"')
        .ignore_then(
            just("\\\\").to(StrPart::Lit("\\\\".to_string()))
                .or(
                    just("${")
                        .ignore_then(expr())
                        .then_ignore(just('}').labelled("変数展開「${」は「}」で閉じてください"))
                        .map(StrPart::Expr)
                )
                .or(
                    just('\\')
                        .then(
                            any().filter(|&c: &char| c != '\r' && c != '\n' && c != '\\' && c != '"' && c != '$')
                                .repeated()
                                .at_least(1)
                                .collect::<String>()
                        )
                        .map(|(bs, rest): (char, String)| {
                            StrPart::Lit(std::iter::once(bs).chain(rest.chars()).collect())
                        })
                )
                .or(
                    any().filter(|&c: &char| c != '\r' && c != '\n' && c != '$' && c != '\\' && c != '"')
                        .repeated()
                        .at_least(1)
                        .collect::<String>()
                        .map(StrPart::Lit)
                )
                .repeated()
                .collect::<Vec<_>>()
        )
        .then_ignore(just('"').labelled("インデックスキーの文字列は「\"」で閉じてください"))
        .map(Expr::InterpolatedStr);

    ident().map(PathSegment::Key)
        .then(
            just('.')
                .ignore_then(ident().map(PathSegment::Key))
            .or(
                just('[')
                    .ignore_then(ws())
                    .ignore_then(
                        index_key
                            .or(str_lit())
                            .or(number())
                    )
                    .then_ignore(ws())
                    .then_ignore(just(']').labelled("インデックスアクセスは「]」で閉じてください"))
                    .map(PathSegment::Index)
            )
            .repeated()
            .collect::<Vec<_>>()
        )
        .map(|(first, rest)| {
            let mut v = vec![first];
            v.extend(rest);
            v
        })
}

fn assign_op<'a>() -> impl Parser<'a, &'a str, AssignOp, extra::Err<Rich<'a, char>>> + Clone {
    just("?=").to(AssignOp::SetIfNull)
        .or(just("+=").to(AssignOp::Add))
        .or(just("-=").to(AssignOp::Sub))
        .or(just("*=").to(AssignOp::Mul))
        .or(just("/=").to(AssignOp::Div))
        .or(just("%=").to(AssignOp::Mod))
        .or(just("=").to(AssignOp::Set))
        .labelled("代入演算子（=、+=、-= など）が必要です")
}

fn global_stmt<'a>() -> impl Parser<'a, &'a str, Stmt, extra::Err<Rich<'a, char>>> + Clone {
  keyword("global")  
        .ignore_then(ws())
        .ignore_then(path_segments())
        .then_ignore(ws())
        .then(assign_op())
        .then_ignore(ws())
        .then(expr())
        .map(|((path, op), val)| Stmt::Global(path, op, val))
}

fn let_stmt<'a>() -> impl Parser<'a, &'a str, Stmt, extra::Err<Rich<'a, char>>> + Clone {
      keyword("let")                          // just("let") から変更
        .ignore_then(ws())
        .ignore_then(ident().labelled("let文には変数名が必要です"))
        .then_ignore(ws())
        .then_ignore(just('=').labelled("「let 変数名 = 値」の「=」が必要です"))
        .then_ignore(ws())
        .then(expr().labelled("「let 変数名 = 値」の値が必要です"))
        .map(|(name, val)| Stmt::Let(name, val))
}

fn assign_stmt<'a>() -> impl Parser<'a, &'a str, Stmt, extra::Err<Rich<'a, char>>> + Clone {
    path_segments()
        .then_ignore(ws())
        .then(assign_op())
        .then_ignore(ws())
        .then(expr())
        .map(|((path, op), val)| Stmt::Assign(path, op, val))
}

// ── stmt ─────────────────────────────────────────────────

fn stmt<'a>() -> impl Parser<'a, &'a str, Spanned<Stmt>, extra::Err<Rich<'a, char>>> + Clone {
    recursive(|stmt| {
        let block = just('{')
            .ignore_then(ws_nl())
            .ignore_then(
                stmt.clone()
                    .then_ignore(ws_nl())
                    .repeated()
                    .collect::<Vec<_>>()
            )
            .then_ignore(just('}').labelled("ブロックを「}」で閉じてください"))
            .then_ignore(ws_nl());
    let func_def = keyword("func")           // just("func") から変更
    .ignore_then(ws())
            .ignore_then(ident().labelled("func定義には関数名が必要です"))
            .then_ignore(ws())
            .then_ignore(just('(').labelled("func定義のパラメータリストは「(」で始めます"))
            .then(
                ident()
                    .separated_by(just(',').padded_by(ws()))
                    .allow_trailing()
                    .collect::<Vec<_>>()
            )
            .then_ignore(just(')').labelled("func定義のパラメータリストは「)」で閉じてください"))
            .then_ignore(ws())
            .then(block.clone())
            .map(|((name, params), body)| Stmt::FuncDef { name, params, body });

      // return_stmt
let return_stmt = keyword("return")       // just("return") から変更
    .ignore_then(ws())
            .ignore_then(expr().or_not())
            .map(|e| Stmt::Return(e.unwrap_or(Expr::Str(String::new()))));

        // for_loop
        let for_loop = keyword("for")             // just("for") から変更
    .ignore_then(ws())
            .ignore_then(just('(').labelled("for文は「(」で始めます"))
            .ignore_then(ws())
       .ignore_then(
        keyword("let")                     // ★ここも
            .ignore_then(ws())
                    .ignore_then(ident())
                    .then_ignore(ws())
                    .then_ignore(just('='))
                    .then_ignore(ws())
                    .then(expr())
                    .map(|(name, val)| Stmt::Let(name, val)),
            )
            .then_ignore(ws())
            .then_ignore(just(';').labelled("for文の初期化式の後に「;」が必要です"))
            .then_ignore(ws())
            .then(expr())
            .then_ignore(ws())
            .then_ignore(just(';').labelled("for文の条件式の後に「;」が必要です"))
            .then_ignore(ws())
            .then(
                ident()
                    .then_ignore(ws())
                    .then(
                        just("++").to(AssignOp::Add)
                            .or(just("--").to(AssignOp::Sub))
                            .or(just("+=").to(AssignOp::Add))
                            .or(just("-=").to(AssignOp::Sub)),
                    )
                    .then(ws().ignore_then(expr()).or_not())
                    .map(|((name, op), val)| {
                        let v = val.unwrap_or(Expr::Number(1.0));
                        Stmt::Global(vec![PathSegment::Key(name)], op, v)
                    }),
            )
            .then_ignore(ws())
            .then_ignore(just(')').labelled("for文の「(」は「)」で閉じてください"))
            .then_ignore(ws())
            .then(block.clone())
            .map(|(((init, cond), step), body)| Stmt::For {
                init: Box::new(init),
                cond,
                step: Box::new(step),
                body,
            });
let foreach_loop = keyword("foreach")     // just("foreach") から変更
    .ignore_then(ws())
            .ignore_then(expr())
            .then_ignore(ws())
               .then_ignore(keyword("as").labelled("「foreach 式 as キー」の「as」が必要です"))
            .then_ignore(ws())
            .then(ident().labelled("「foreach 式 as キー」のキー変数名が必要です"))
            .then(
                ws().ignore_then(just(','))
                    .ignore_then(ws())
                    .ignore_then(ident())
                    .or_not(),
            )
            .then_ignore(ws())
            .then(block.clone())
            .map(|(((col, key), val), body)| Stmt::ForEach {
                collection: col,
                key,
                value: val,
                body,
            });
let while_loop = keyword("while")         // just("while") から変更
    .ignore_then(ws())
            .ignore_then(just('(').labelled("while文の条件式は「(」で始めます"))
            .ignore_then(ws())
            .ignore_then(expr())
            .then_ignore(ws())
            .then_ignore(just(')').labelled("while文の条件式は「)」で閉じてください"))
            .then_ignore(ws())
            .then(block.clone())
            .map(|(cond, body)| Stmt::While(cond, body));

        let if_stmt = recursive(|if_stmt| {
         keyword("if")               // if_stmt内
                .ignore_then(ws())
                .ignore_then(just('(').labelled("if文の条件式は「(」で始めます"))
                .ignore_then(ws())
                .ignore_then(expr())
                .then_ignore(ws())
                .then_ignore(just(')').labelled("if文の条件式は「)」で閉じてください"))
                .then_ignore(ws())
                .then(block.clone())
                .then(
                    keyword("else")
                        .ignore_then(ws())
                        .ignore_then(
                            // 「else if ...」は入れ子のif_stmt（Stmt::If単体）を
                            // else_bodyのVec<Spanned<Stmt>>に合わせて1要素でラップする。
                            // このSpannedのlineはIf自体の行であり、check_stmt側では
                            // Stmt::Ifノード自体の行番号を参照しないため使われない
                            // （実際に報告されるのは中のthen_body/else_body各文の行）。
                            if_stmt.map(|s| vec![Spanned { node: s, line: 0 }]).or(block.clone()),
                        )
                        .or_not(),
                )
                .map(|((c, then_body), else_body)| Stmt::If(c, then_body, else_body))
        });
        

        // match文（インライン）
        let match_stmt = {
   
                    // match_stmt の arm 定義の直前
let pattern = just('_')
    .then_ignore(
        any().filter(|c: &char| {
            c.is_alphanumeric() || *c == '_' || (*c as u32) > 0x7F
        })
        .not()
    )
    .to(MatchPattern::Wildcard)
    .or(expr().map(MatchPattern::Value));

let patterns = pattern.clone()
    .then(
        ws().ignore_then(just('|'))
            .then_ignore(ws())
            .ignore_then(pattern)
            .repeated()
            .collect::<Vec<_>>()
    )
    .map(|(first, rest)| {
        let mut v = vec![first];
        v.extend(rest);
        v
    });
            let arm = patterns
                .then_ignore(ws())
                .then_ignore(just("=>").labelled("matchのパターンの後に「=>」が必要です"))
                .then_ignore(ws_nl())
                .then(
                    just('{').labelled("matchアームの本体は「{」で始めます")
                        .ignore_then(ws_nl())
                        .ignore_then(
                            stmt.clone()
                                .then_ignore(ws_nl())
                                .repeated()
                                .collect::<Vec<_>>()
                        )
                        .then_ignore(just('}').labelled("matchアームは「}」で閉じてください"))
                        .then_ignore(ws_nl())
                )
                .map(|(patterns, body)| MatchArm { patterns, body });

           keyword("match")
                .ignore_then(ws())
                .ignore_then(expr())
                .then_ignore(ws_nl())
                .then_ignore(just('{').labelled("match文の本体は「{」で始めます"))
                .then_ignore(ws_nl())
                .then(arm.repeated().collect::<Vec<_>>())
                .then_ignore(ws_nl())
                .then_ignore(just('}').labelled("match文は「}」で閉じてください"))
                .map(|(expr, arms)| Stmt::Match { expr, arms })
        };

   let call_stmt = keyword("call") 
    .ignore_then(ws())
    .ignore_then(var_path().labelled("call文にはイベント名または関数名が必要です"))
    .map(|path| Stmt::Call(Expr::Var(path)));

let expr_stmt = ident()
    .then_ignore(just('('))
    .then(
        expr()
            .separated_by(just(',').padded_by(ws_nl()))
            .allow_trailing()
            .collect::<Vec<_>>()
    )
    .then_ignore(just(')').labelled("関数呼び出しは「)」で閉じてください"))
    .map(|(name, args)| Stmt::Call(Expr::Call(name, args)));
        choice((
            for_loop,
            foreach_loop,
            while_loop,
            if_stmt,
            match_stmt,
            func_def,
            return_stmt,
        keyword("break").to(Stmt::Break),
        keyword("continue").to(Stmt::Continue),
            call_stmt,
            expr_stmt,
            global_stmt(),
            let_stmt(),
            assign_stmt(),
            dialogue(),
            // ★変更: フォールバックを validate にして span 付きでエラーをemit
            //   __skip__ は into_result() が Err を返すため codegen には到達しない
            any().filter(|&c: &char| c != '\n' && c != '\r' && c != '}')
                .repeated()
                .at_least(1)
                .collect::<String>()
                .validate(|s, e, emitter| {
                    emitter.emit(Rich::custom(
                        e.span(),
                        format!("認識できない文です: 「{}」", s),
                    ));
                    Stmt::Let("__skip__".to_string(), Expr::Str(s))
                })
        ))
        // ★行番号は、preprocess後のソース上でのバイトオフセットとして
        //   ひとまず持たせておく（この時点ではまだ「preprocess後の行番号」で
        //   「元ソースの行番号」ではない）。実際の行番号への変換は
        //   resolve_stmt_lines() でパース完了後にまとめて行う。
        .map_with(|stmt, e| Spanned { node: stmt, line: e.span().start as u32 })
    })
}

// ── talk ─────────────────────────────────────────────────

pub fn talk<'a>() -> impl Parser<'a, &'a str, Talk, extra::Err<Rich<'a, char>>> + Clone {
    ws_nl()
        .ignore_then(ident())
        .then_ignore(ws_nl())
        // if(条件式) をオプションで受け取る
        .then(
            keyword("if") 
                .ignore_then(ws())
                .ignore_then(just('(').labelled("トーク条件は「if(」で始めます"))
                .ignore_then(ws())
                .ignore_then(expr())
                .then_ignore(ws())
                .then_ignore(just(')').labelled("トーク条件の「(」は「)」で閉じてください"))
                .then_ignore(ws_nl())
                .or_not()
        )
        .then_ignore(just("=>").labelled("イベント名の後に「=>」が必要です"))
        .then_ignore(ws_nl())
        .then(
            just('{').labelled("トーク定義は「{」で始めます")
                .ignore_then(ws_nl())
                .ignore_then(
                    stmt()
                        .then_ignore(ws_nl())
                        .repeated()
                        .collect::<Vec<_>>()
                )
                .then_ignore(ws_nl())
                .then_ignore(just('}').labelled("トーク定義は「}」で閉じてください"))
                .then_ignore(ws_nl())
        )
        .map(|((event, cond), body)| Talk { event, cond, body })
}
// ── preprocess（変更なし）────────────────────────────────

// ──────────────────────────────────────────────────────────
// 修正: preprocess
//   変更点: let/global 限定だった値なし代入チェックを
//           ends_with_bare_assign に置き換えて全種に対応。
// ──────────────────────────────────────────────────────────


///   - コメント行
fn ends_with_bare_assign(s: &str) -> bool {
    if s.starts_with("//") || s.starts_with("/*") {
        return false;
    }
    if s.contains("=>") {
        return false;
    }
    let t = s.trim_end();
    if !t.ends_with('=') {
        return false;
    }
    !t.ends_with("==")
        && !t.ends_with("!=")
        && !t.ends_with("?=")
        && !t.ends_with("+=")
        && !t.ends_with("-=")
        && !t.ends_with("*=")
        && !t.ends_with("/=")
        && !t.ends_with("%=")
}
/// コード行（文字列リテラルの外側）における `{`/`(`/`[` の最大ネスト深さ。
/// exprやstmtは再帰下降パーサーで実装されており構文的なネストの深さに
/// 上限がないため、悪意ある/壊れた辞書ファイル（他人が配布したゴーストの
/// 辞書を読み込むことが日常的な文化圏である以上、これは現実的な脅威）が
/// `((((((...))))))`のような深い括弧のネストを仕込むと、Rustのネイティブ
/// スタックを使い果たしてスタックオーバーフローになる。スタックオーバー
/// フローはpanic=unwindでもcatch_unwindでは捕捉できず、無条件にプロセスを
/// 強制終了させる。ここでプレーンなテキストスキャンとして事前に深さを
/// 数えて打ち切ることで、パーサー本体（chumsky）には一切手を入れずに
/// スタックオーバーフローを未然に防ぐ。
/// （chumsky側のrecursiveな各パーサーをカスタムコンビネータで包んで
/// 深さを数える実装も試したが、コンパイル時間が数分から20分超に
/// 悪化したため採用しなかった）
const MAX_NESTING_DEPTH: u32 = 200;

/// preprocess の結果。セリフや継続行を1行に結合した出力ソースと、
/// 出力の各行が元ソースの何行目に由来するかの対応表を持つ。
pub struct PreprocessResult {
    pub src: String,
    /// line_map[i] = 出力の (i+1) 行目が由来する元ソースの行番号。
    /// 1行に複数の元行が結合された場合は、その行の「先頭」の元行番号を持つ。
    line_map: Vec<u32>,
}

impl PreprocessResult {
    /// 出力側の行番号（1-indexed）から元ソースの行番号を引く。
    pub fn resolve_line(&self, output_line: u32) -> u32 {
        self.line_map
            .get((output_line as usize).saturating_sub(1))
            .copied()
            .unwrap_or(output_line)
    }
}

pub fn preprocess(src: &str) -> Result<PreprocessResult, String> {
    let mut out = String::new();
    let mut line_map: Vec<u32> = Vec::new();
    let mut buf: Option<(String, u32)> = None;
    let mut prev_chara: Option<String> = None;
    let mut brace_stack: Vec<BraceKind> = Vec::new();   // ★ brace_depth → brace_stack
    let mut nesting_depth: u32 = 0;

    for (line_idx, line) in src.lines().enumerate() {
        let line_num = (line_idx + 1) as u32;
        let trimmed = line.trim();


        if ends_with_bare_assign(trimmed) {
            if let Some((b, start_line)) = buf.take() {
                out.push_str(&b);
                out.push('\n');
                line_map.push(start_line);
            }
            return Err(format!(
                "{}行目: 代入する値がありません。「{}」の後に値を書いてください。",
                line_num, trimmed
            ));
        }
// is_in_map は「この行が始まる前のスタック状態」で決める。
        let is_in_map = matches!(brace_stack.last(), Some(BraceKind::Map));

        let kind = if trimmed == ";" {
            LineKind::Separator
        } else if !is_in_map && is_dialogue_line(trimmed) {
            LineKind::Dialogue
        } else if trimmed.starts_with('\\') && buf.is_some() && !is_control_backslash(trimmed) {
            LineKind::TagAppend
        } else if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with('}')
            || trimmed.starts_with('{')
            || trimmed.starts_with('\\')
            || trimmed.starts_with("if")
            || trimmed.starts_with("else")
            || trimmed.starts_with("for")
            || trimmed.starts_with("while")
            || trimmed.starts_with("match")
            || trimmed.starts_with("func")
            || trimmed.starts_with("break")
            || trimmed.starts_with("continue")
            || trimmed.starts_with("let")
            || trimmed.starts_with("global")
            || trimmed.starts_with("return")
            || trimmed.starts_with("call")
            || trimmed.contains("=>")
        {
            LineKind::Code
        } else if buf.is_some() {
            LineKind::Continuation
        } else {
            LineKind::Bare
        };

        // セリフ本文はただの文字列なので、中の「{」はブロックや
        // マップの開閉ではない。走査してしまうと
        // 「湊: 括弧「{」の話」の一行でbrace_stackが壊れ、
        // 以降 is_in_map が真に張り付いてセリフ結合が効かなくなる。
        // さらに талの閉じ「}」がMapを剥がすため、ズレが後続へ残る。
        if matches!(kind, LineKind::Code | LineKind::Bare) {
            let events = scan_braces(trimmed, &mut nesting_depth)
                .map_err(|e| format!("{}行目: {}", line_num, e))?;
            for ev in events {
                match ev {
                    BraceEvent::Open(k) => brace_stack.push(k),
                    BraceEvent::Close => { brace_stack.pop(); }
                }
            }
        }

        match kind {
            LineKind::Separator => {
                if let Some((b, start_line)) = buf.take() {
                    out.push_str(&b);
                    out.push('\n');
                    line_map.push(start_line);
                }
                prev_chara = None;
            }
            LineKind::Dialogue => {
                let line_clean = if line.trim_end().ends_with(';') {
                    line.trim_end().trim_end_matches(';').to_string()
                } else {
                    line.to_string()
                };
                let cur_chara = extract_chara(trimmed);
                if let Some((b, start_line)) = buf.take() {
                    if prev_chara.as_deref() == cur_chara.as_deref() {
                        out.push_str(b.trim_end());
                        out.push_str("\\n");
                        out.push('\n');
                    } else {
                        out.push_str(&b);
                        out.push('\n');
                    }
                    line_map.push(start_line);
                }
                prev_chara = cur_chara;
                buf = Some((line_clean, line_num));
            }
            LineKind::TagAppend | LineKind::Continuation => {
                if let Some((ref mut b, _)) = buf {
                    b.push_str("\\n");
                    b.push_str(trimmed);
                }
            }
            LineKind::Code => {
                if let Some((b, start_line)) = buf.take() {
                    out.push_str(&b);
                    out.push('\n');
                    line_map.push(start_line);
                }
                prev_chara = None;
                out.push_str(line);
                out.push('\n');
                line_map.push(line_num);
            }
            LineKind::Bare => {
                out.push_str(line);
                out.push('\n');
                line_map.push(line_num);
            }
        }
    }
    if let Some((b, start_line)) = buf {
        out.push_str(&b);
        out.push('\n');
        line_map.push(start_line);
    }
    Ok(PreprocessResult { src: out, line_map })
}

//// 文字列リテラルの外側にある { と } の数を数える
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BraceKind { Block, Map }
/// preprocessが一行をどう扱うか。
/// scan_bracesを呼ぶかどうかの判定と、実際の処理で
/// 同じ条件を二度書かないために一度だけ決める。
enum LineKind {
    Separator,     // ";" 単独
    Dialogue,      // セリフの先頭行
    TagAppend,     // 直前の発話に連結する \q[...] 等
    Code,          // ブロック・制御構文など、構文として読む行
    Continuation,  // 直前の発話の続き（コロンなしの本文）
    Bare,          // bufが無い状態で来た単独の行
}
enum BraceEvent { Open(BraceKind), Close }

/// 文字列リテラル・${...}補間の外側にある '{' '}' だけを対象に、
/// それぞれが「ブロック」（if/for/while/foreach/func/match/elseの本体、
/// トーク定義・matchアームの本体）か「マップリテラル」かを判定して
/// 開閉イベント列を返す。
/// ネスト深さを1増やし、上限を超えたらエラーにする
fn bump_nesting_depth(depth: &mut u32) -> Result<(), String> {
    *depth += 1;
    if *depth > MAX_NESTING_DEPTH {
        Err("構文のネストが深すぎます（{}・()・[]の入れ子を減らしてください）".to_string())
    } else {
        Ok(())
    }
}

fn scan_braces(trimmed: &str, depth: &mut u32) -> Result<Vec<BraceEvent>, String> {
    let mut events = Vec::new();
    let mut in_double = false;
    let mut in_single = false;
    let mut interp_depth: i32 = 0; // ${ ... } の中にいるか
    let chars: Vec<char> = trimmed.chars().collect();
    let mut i = 0;

    // "} else" / "else" / "if" / "for"（foreach含む）/ "while" /
    // "func" / "match" で始まる行かどうか（先頭の "}" は無視）
    let stripped_head = trimmed.strip_prefix('}').map(str::trim_start).unwrap_or(trimmed);
    let starts_with_block_kw =
        stripped_head.starts_with("if")
        || stripped_head.starts_with("else")
        || stripped_head.starts_with("for")     // foreach もこれで拾える
        || stripped_head.starts_with("while")
        || stripped_head.starts_with("func")
        || stripped_head.starts_with("match");

    while i < chars.len() {
        let c = chars[i];
        match c {
            '/' if !in_double && !in_single && interp_depth == 0 => {
                if chars.get(i + 1) == Some(&'/') { break; }
            }
            '"' if !in_single && interp_depth == 0 => { in_double = !in_double; }
            '\'' if !in_double && interp_depth == 0 => { in_single = !in_single; }
            '$' if !in_double && !in_single && chars.get(i + 1) == Some(&'{') => {
                interp_depth += 1;
                i += 2; // "${" を読み飛ばす
                continue;
            }
            '{' if !in_double && !in_single => {
                bump_nesting_depth(depth)?;
                if interp_depth > 0 {
                    interp_depth += 1;
                } else {
                    // この '{' より前のテキストから種類を判定
                    let before: String = chars[..i].iter().collect();
                    let before = before.trim_end();
                    let kind = if before.ends_with("=>") || before.ends_with(')') || starts_with_block_kw {
                        BraceKind::Block
                    } else {
                        BraceKind::Map
                    };
                    events.push(BraceEvent::Open(kind));
                }
            }
            '}' if !in_double && !in_single => {
                *depth = depth.saturating_sub(1);
                if interp_depth > 0 {
                    interp_depth -= 1;
                } else {
                    events.push(BraceEvent::Close);
                }
            }
            '(' | '[' if !in_double && !in_single => {
                bump_nesting_depth(depth)?;
            }
            ')' | ']' if !in_double && !in_single => {
                *depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        i += 1;
    }
    Ok(events)
}
fn is_dialogue_line(s: &str) -> bool {
    let s = if s.starts_with('[') {
        s.find(']').map(|i| &s[i+1..]).unwrap_or(s)
    } else { s };
    // 半角コロンで判定（全角コロン「：」は除外）
    let colon_pos = match s.find(':') {
        Some(p) => p,
        None => return false,
    };
    let before_colon = &s[..colon_pos];
    !before_colon.is_empty() && before_colon.chars().all(|c| {
        c.is_alphanumeric() || c == '_' || (c as u32) > 0x7F
    }) && !before_colon.contains('：')  // 全角コロンを含む場合は除外
}
fn is_control_backslash(s: &str) -> bool {
    s.starts_with("\\-") || s.starts_with("\\e") || s.starts_with("\\![")
}

fn extract_chara(s: &str) -> Option<String> {
    let s = if s.starts_with('[') {
        s.find(']').map(|i| &s[i + 1..]).unwrap_or(s)
    } else { s };
    let colon_pos = s.find(':')?;
    let name = &s[..colon_pos];
    if name.is_empty() { None } else { Some(name.to_string()) }
}

// ── LoadError ────────────────────────────────────────────

#[derive(Debug)]
pub enum LoadError {
    ParseError(Vec<String>, PathBuf),
    PreprocessError(String),
}

// ── ファイル読み込み ─────────────────────────────────────

pub fn load_program(
    entry: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Result<
    (Vec<Talk>, Vec<(String, Vec<String>, Vec<Spanned<Stmt>>)>, Vec<(Vec<PathSegment>, AssignOp, Expr)>),
    LoadError
> {
    let canonical = entry.canonicalize().unwrap_or_else(|_| entry.to_path_buf());
    if visited.contains(&canonical) {
        return Ok((vec![], vec![], vec![]));
    }
    visited.insert(canonical.clone());

    let src = std::fs::read_to_string(entry)
        .map_err(|e| LoadError::PreprocessError(format!("ファイルが読み込めません: {}", e)))?;

    append_log!("before preprocess");
    let pre = match preprocess(&src) {
        Ok(r) => r,
        Err(e) => return Err(LoadError::PreprocessError(e)),
    };
    let src: &str = &pre.src;
    append_log!("after preprocess");

    let base_dir = entry.parent().unwrap_or(Path::new("."));
#[cfg(debug_assertions)]
{
    let log_name = format!("preprocess_{}.log",entry.file_stem().unwrap_or_default().to_string_lossy());
    let log_path = entry.parent().unwrap_or(Path::new(".")).join(log_name);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true).write(true).truncate(true)
        .open(&log_path)
    {
        let _ = f.write_all(src.as_bytes());
    }
}
    append_log!("before parse");

    let file_name = entry.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut items = program_with_include()
        .parse(src)
        .into_result()
        .map_err(|errors| {
            let msgs: Vec<String> = errors.iter()
                .map(|e| rich_to_japanese(e, &pre, &file_name))
                .collect();
            LoadError::ParseError(msgs, entry.to_path_buf())
        })?;
    resolve_program_item_lines(&mut items, src, &pre);

    append_log!("after parse");

    let mut talks = vec![];
    let mut funcs = vec![];
    let mut globals = vec![];

    for item in items {
        match item {
            ProgramItem::Include(path) => {
                let child = base_dir.join(&path);
                append_log!(&format!("include: {:?}" , child));
                let (mut ct, mut cf, mut cg) = load_program(&child, visited)?;
                talks.append(&mut ct);
                funcs.append(&mut cf);
                globals.append(&mut cg);
            }
            ProgramItem::Talk(talk) => talks.push(talk),
            ProgramItem::FuncDef { name, params, body } => funcs.push((name, params, body)),
            ProgramItem::Global(path, op, expr) => globals.push((path, op, expr)),
        }
    }
    Ok((talks, funcs, globals))
}

// ── エラーメッセージ日本語化 ──────────────────────────────

fn rich_to_japanese(e: &Rich<char>, pre: &PreprocessResult, file_name: &str) -> String {
    let pos = e.span().start as u32;
    let output_line = line_at_offset(pos, &pre.src);
    let line = pre.resolve_line(output_line);
    let msg = reason_to_japanese(e.reason());
    format!("{}の{}行目: {}", file_name, line, msg)
}
    
    fn reason_to_japanese(reason: &chumsky::error::RichReason<char>) -> String {
    use chumsky::error::{RichReason, RichPattern};

    match reason {
        // .labelled("日本語") の出力はここに入る → そのまま日本語で出力
        RichReason::Custom(s) => s.to_string(),

        RichReason::ExpectedFound { expected, found } => {
            let found_str = match found {
                None => "ファイル末尾".to_string(),
                Some(f) => {
                    let d = format!("{:?}", f);
                    extract_char_from_debug(&d)
                        .map(|c| format!("「{}」", c))
                        .unwrap_or_else(|| "不明な文字".to_string())
                }
            };

            let exp_count = expected.len();
            if exp_count == 0 {
                return format!("予期しない{}があります", found_str);
            }

            // .labelled("日本語") は RichPattern::Label に入る → 優先して使う
            let labels: Vec<String> = expected.iter()
                .filter_map(|p| match p {
                    RichPattern::Label(s) => Some((*s).to_string()),
                    _ => None,
                })
                .collect();

            if !labels.is_empty() {
                return format!("{}（{}付近）", labels[0], found_str);
            }

            // ラベルなし：トークン一覧を展開
            let tokens: Vec<String> = expected.iter()
                .filter_map(|p| match p {
                    RichPattern::Token(t) => Some(format!("「{:?}」", t)),
                    RichPattern::EndOfInput => Some("ファイル末尾".to_string()),
                    _ => None,
                })
                .collect();

            if exp_count <= 3 {
                format!("{}が必要なところに{}があります", tokens.join("または"), found_str)
            } else {
                format!("{}付近に記述ミスがあります", found_str)
            }
        }
    }
}

    
   fn extract_char_from_debug(s: &str) -> Option<char> {
    let start = s.find('\'')?;
    let rest = &s[start + 1..];
    if rest.starts_with('\\') {
        match rest.chars().nth(1)? {
            'n'  => Some('\n'),
            't'  => Some('\t'),
            '\\' => Some('\\'),
            '\'' => Some('\''),
            'r'  => Some('\r'),
            c    => Some(c),
        }
    } else {
        rest.chars().next()
    }
}

// ── ProgramItem ──────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ProgramItem {
    Include(String),
    Talk(Talk),
    FuncDef { name: String, params: Vec<String>, body: Vec<Spanned<Stmt>> },
    Global(Vec<PathSegment>, AssignOp, Expr),
}

// ── 行番号解決 ───────────────────────────────────────────
// stmt()のmap_withでは、Spanned.lineに「preprocess後のソース上のバイト
// オフセット」を暫定的に入れている（パーサーコンビネータの中では
// テキストを自由に参照できないため）。パースが完了した後、この
// バイトオフセットを実際の行番号に変換し、さらにPreprocessResultの
// resolve_lineを通して元ソースの行番号に変換する。

/// バイトオフセットが、preprocess後のソース上で何行目に当たるかを返す（1-indexed）。
/// rich_to_japaneseと同じ考え方（オフセットより前の改行文字を数える）。
fn line_at_offset(offset: u32, src: &str) -> u32 {
    let pos = (offset as usize).min(src.len());
    src[..pos].chars().filter(|&c| c == '\n').count() as u32 + 1
}

/// Vec<Spanned<Stmt>>を再帰的に辿り、各Spanned.lineを
/// 「preprocess後のバイトオフセット」から「元ソースの行番号」に書き換える。
fn resolve_stmt_lines(stmts: &mut [Spanned<Stmt>], src: &str, pre: &PreprocessResult) {
    for spanned in stmts.iter_mut() {
        let preprocessed_line = line_at_offset(spanned.line, src);
        spanned.line = pre.resolve_line(preprocessed_line);
        match &mut spanned.node {
            Stmt::If(_, then_body, else_body) => {
                resolve_stmt_lines(then_body, src, pre);
                if let Some(eb) = else_body {
                    resolve_stmt_lines(eb, src, pre);
                }
            }
            Stmt::For { body, .. }
            | Stmt::ForEach { body, .. }
            | Stmt::FuncDef { body, .. } => resolve_stmt_lines(body, src, pre),
            Stmt::While(_, body) => resolve_stmt_lines(body, src, pre),
            Stmt::Match { arms, .. } => {
                for arm in arms.iter_mut() {
                    resolve_stmt_lines(&mut arm.body, src, pre);
                }
            }
            _ => {}
        }
    }
}

/// program_with_include()の結果全体について、含まれるTalk/FuncDefの
/// 本体すべての行番号を元ソースの行番号に変換する。
pub fn resolve_program_item_lines(items: &mut [ProgramItem], src: &str, pre: &PreprocessResult) {
    for item in items.iter_mut() {
        match item {
            ProgramItem::Talk(talk) => resolve_stmt_lines(&mut talk.body, src, pre),
            ProgramItem::FuncDef { body, .. } => resolve_stmt_lines(body, src, pre),
            ProgramItem::Include(_) | ProgramItem::Global(_, _, _) => {}
        }
    }
}

fn include_directive<'a>() -> impl Parser<'a, &'a str, ProgramItem, extra::Err<Rich<'a, char>>> + Clone {
     keyword("include")
        .ignore_then(ws())
        .ignore_then(just('"').labelled("include文のファイルパスは「\"」で囲んでください"))
        .ignore_then(
            any().filter(|&c: &char| c != '"')
                .repeated()
                .collect::<String>()
        )
        .then_ignore(just('"').labelled("include文のファイルパスは「\"」で閉じてください"))
        .map(ProgramItem::Include)
}

pub fn program_with_include<'a>() -> impl Parser<'a, &'a str, Vec<ProgramItem>, extra::Err<Rich<'a, char>>> {
    let top_global = global_stmt()
        .map(|s| match s {
            Stmt::Global(path, op, expr) => ProgramItem::Global(path, op, expr),
            _ => unreachable!(),
        });

    let top_func = keyword("func")  
        .ignore_then(ws())
        .ignore_then(ident())
        .then_ignore(ws())
        .then_ignore(just('('))
        .then(
            ident()
                .separated_by(just(',').padded_by(ws()))
                .allow_trailing()
                .collect::<Vec<_>>()
        )
        .then_ignore(just(')'))
        .then_ignore(ws())
        .then(
            just('{')
                .ignore_then(ws_nl())
                .ignore_then(
                    stmt()
                        .then_ignore(ws_nl())
                        .repeated()
                        .collect::<Vec<_>>()
                )
                .then_ignore(ws_nl())
                .then_ignore(just('}'))
                .then_ignore(ws_nl())
        )
        .map(|((name, params), body)| ProgramItem::FuncDef { name, params, body });

    ws_nl()
        .ignore_then(
            include_directive()
                .or(top_global)
                .or(top_func)
                .or(talk().map(ProgramItem::Talk))
        )
        .repeated()
        .collect::<Vec<_>>()
        .then_ignore(ws_nl())
        .then_ignore(end().labelled("ファイルの末尾に予期しない内容があります"))
}


//1




//3

#[test]
fn test_is_dialogue_line_zenkaku() {
    assert!(!is_dialogue_line("日の出：${sun_hour(sunrise)}時"));
    assert!(is_dialogue_line("湊: テスト"));
}

#[test]
fn test_preprocess_calendar2() {
    let src = r#"OnShowAstroInfo => {
    湊: ${city}の今日の天文情報です。
      日の出：${sun_hour(sunrise)}時分
}"#;
    let result = preprocess(src).expect("preprocess failed");
    println!("result:\n{}", result.src);
}
#[test]
fn test_is_dialogue_line_debug() {
    let lines = vec![
        "日の出：${sun_hour(sunrise)}時${sun_minute(sunrise)}分",
        "日の入り：${sun_hour(sunset)}時${sun_minute(sunset)}分",
        "月齢：${floor(moon_age)}（${moon_emoji}${moon_phase}）",
    ];
    for line in &lines {
        println!("is_dialogue_line({:?}) = {}", line, is_dialogue_line(line));
        println!("contains => : {}", line.contains("=>"));
        println!("starts_with \\ : {}", line.starts_with('\\'));
    }
}

#[test]
fn test_preprocess_dialogue_merge_inside_if_block() {
    let src = r#"OnBoot => {
    if (x) {
        湊: A
        湊: B
    }
}"#;
    let out = preprocess(src).expect("preprocess failed").src;
    // A行末に \n（SAKURAの改行タグ）が挿入され、if の中で正しくマージされる
    assert!(out.contains("湊: A\\n"));
    println!("{}", out);
}

#[test]
fn test_preprocess_multiline_dialogue_inside_for_loop() {
    // 継続行（コロンなし）が if/for の中でも解析エラーにならず buf に連結される
    let src = r#"OnBoot => {
    for (let i = 0; i < 3; i++) {
        湊: ほげ
        ふが
    }
}"#;
    let out = preprocess(src).expect("preprocess failed").src;
    assert!(out.contains("ほげ\\nふが"));
}



#[test]
fn test_preprocess_map_still_excluded_inside_nested_block() {
    // ブロックの中にマップリテラルがあっても、マップの中身は
    // セリフとして誤認識されない（is_in_map が正しく効く）
    let src = r#"OnBoot => {
    if (x) {
        let m = {
            a: 1,
            b: 2,
        }
        global save.count = len(m)
    }
}"#;
    let out = preprocess(src).expect("preprocess failed").src;
    // a: 1 / b: 2 がダイアログ結合（\n連結）の対象になっていないことを確認
    assert!(!out.contains("a: 1\\n"));
    let talks = crate::parser::program_with_include()
        .parse(&*out);
    assert!(talks.into_result().is_ok(), "map literal inside nested block should still parse");
}

#[test]
fn test_preprocess_interpolation_does_not_corrupt_brace_stack() {
    // ${...} の } が誤って Close と数えられていないか
    let src = r#"OnBoot => {
    if (x) {
        湊: ${save.count}回目
        湊: 続き
    }
}"#;
    let out = preprocess(src).expect("preprocess failed").src;
    assert!(out.contains("回目\\n"), "brace_stack corrupted by ${{}}: {}", out);
}



#[test]
fn test_not_and_precedence_structural() {
    let src = r#"OnBoot => {
    if (!x && y) {
        湊: A
    } else {
        湊: B
    }
}"#;
    let items = program_with_include().parse(src).into_result().expect("parse failed");
    let talk = items.iter().find_map(|item| match item {
        ProgramItem::Talk(t) => Some(t),
        _ => None,
    }).unwrap();

    match &talk.body[0].node {
        Stmt::If(cond, _, _) => match cond {
            // !x && y は And(Not(x), y) であるべき（Not(And(x, y)) ではない）
            Expr::And(lhs, rhs) => {
                assert!(matches!(**lhs, Expr::Not(_)), "左辺はNot(x)であるべき: {:?}", lhs);
                assert!(matches!(**rhs, Expr::Var(_)), "右辺はyであるべき: {:?}", rhs);
            }
            other => panic!("最上位はAndであるべきだが: {:?}", other),
        },
        other => panic!("If文であるべきだが: {:?}", other),
    }


}
#[test]
fn test_keyword_does_not_swallow_identifier_prefix_true() {
    // "truename = 1" が Bool(true) + 独立した "name" に誤分割されず、
    // 単一の識別子への代入としてパースされることを確認
    let src = r#"OnBoot => {
    truename = 1
    湊: ${truename}
}"#;
    let items = program_with_include().parse(src).into_result();
    assert!(items.is_ok(), "「truename」を予約語trueの前方一致で誤分割している: {:?}", items.err());
}

#[test]
fn test_keyword_does_not_swallow_identifier_prefix_break() {
    let src = r#"OnBoot => {
    let breakfast = 'あさごはん'
    湊: ${breakfast}
}"#;
    let items = program_with_include().parse(src).into_result();
    assert!(items.is_ok(), "「breakfast」を予約語breakの前方一致で誤分割している: {:?}", items.err());
}

#[test]
fn test_keyword_still_works_as_standalone_token() {
    // キーワード本来の意味は引き続き機能すること（回帰確認）
    let src = r#"OnBoot => {
    for (let i = 0; i < 10; i++) {
        if (i == 3) {
            break
        }
    }
    湊: おわり
}"#;
    let items = program_with_include().parse(src).into_result();
    assert!(items.is_ok(), "{:?}", items.err());
}

#[test]
fn test_surface_number_overflow_does_not_panic() {
    // u32::MAX を超えるサーフェス番号は、パニックせず構文エラーとして
    // 報告されること（かつては .unwrap() でプロセスごとクラッシュしていた）
    let src = r#"OnBoot => {
    [99999999999]湊: こんにちは
}"#;
    let items = program_with_include().parse(src).into_result();
    assert!(items.is_err(), "桁溢れしたサーフェス番号は構文エラーになるべき");
}

#[test]
fn test_deeply_nested_parens_are_rejected_before_parsing() {
    // 悪意ある/壊れた辞書ファイルが極端に深い括弧のネストを仕込んでも、
    // chumskyの再帰下降パーサーに到達する前にpreprocess()の時点で
    // 安全な構文エラーとして打ち切られること（スタックオーバーフロー対策）
    let opens = "(".repeat(300);
    let closes = ")".repeat(300);
    let src = format!("OnBoot => {{\n    let x = {}1{}\n}}", opens, closes);
    let result = preprocess(&src);
    assert!(result.is_err(), "300段の括弧ネストは構文エラーとして拒否されるべき");
}

#[test]
fn test_deeply_nested_blocks_are_rejected_before_parsing() {
    let opens = "if (1) {\n".repeat(300);
    let src = format!("OnBoot => {{\n{}}}", opens);
    let result = preprocess(&src);
    assert!(result.is_err(), "300段のブロックネストは構文エラーとして拒否されるべき");
}

#[test]
fn test_moderately_nested_expr_still_parses_normally() {
    // 深さ制限が現実的な辞書スクリプトの正当なネストまで壊していないことの回帰確認
    let opens = "(".repeat(20);
    let closes = ")".repeat(20);
    let src = format!(
        "OnBoot => {{\n    let x = {}1{}\n    湊: ${{x}}\n}}",
        opens, closes
    );
    let items = program_with_include().parse(&src).into_result();
    assert!(items.is_ok(), "通常のネストまで拒否している: {:?}", items.err());
}



        #[test]
fn test_dialogue_containing_open_brace_does_not_corrupt_brace_stack() {
    let src = r#"OnBoot => {
    湊: 開き括弧「{」の話
    湊: 続き
}"#;
    let out = preprocess(src).expect("preprocess failed").src;
    assert!(out.contains("話\\n"), "セリフ本文の「{{」でbrace_stackが壊れている: {}", out);
    assert!(
        program_with_include().parse(&*out).into_result().is_ok(),
        "パースできない: {}", out
    );
}

#[test]
fn test_dialogue_containing_close_brace_does_not_pop_block() {
    let src = r#"OnBoot => {
    湊: 閉じ括弧「}」の話
    湊: 続き
}"#;
    let out = preprocess(src).expect("preprocess failed").src;
    assert!(out.contains("話\\n"), "セリフ本文の「}}」でbrace_stackが壊れている: {}", out);
}

#[test]
fn test_dialogue_with_brace_does_not_leak_into_next_talk() {
    // 壊れたスタックが次のトークまで持ち越されないこと
    let src = r#"OnBoot => {
    湊: 「{」
}
OnClose => {
    湊: A
    湊: B
}"#;
    let out = preprocess(src).expect("preprocess failed").src;
    assert!(out.contains("湊: A\\n"), "次のトークまでズレが残っている: {}", out);
}

#[test]
fn test_line_number_preserved_simple() {
    let src = "OnBoot => {\n    call 存在しない関数()\n}";
    let result = preprocess(src).expect("preprocess failed");
    let output_line = result.src.lines()
        .position(|l| l.contains("call 存在しない関数"))
        .map(|i| i + 1)
        .unwrap();
    let original_line = result.resolve_line(output_line as u32);
    assert_eq!(original_line, 2);
}

#[test]
fn test_line_number_preserved_dialogue_merge() {
    // セリフ本文と継続行が1行に結合されても、由来する元行番号は
    // その結合行の「先頭」（湊: セリフ1のある行）を指すこと。
    let src = r#"OnBoot => {
    湊: セリフ1
    続き
    湊: セリフ2
    call 存在しない関数()
}"#;
    let result = preprocess(src).expect("preprocess failed");

    let merged_output_line = result.src.lines()
        .position(|l| l.contains("セリフ1"))
        .map(|i| i + 1)
        .unwrap();
    assert_eq!(result.resolve_line(merged_output_line as u32), 2);

    let call_output_line = result.src.lines()
        .position(|l| l.contains("call 存在しない関数"))
        .map(|i| i + 1)
        .unwrap();
    assert_eq!(result.resolve_line(call_output_line as u32), 5);
}

#[test]
fn test_line_map_len_matches_output_lines() {
    // out に積む行数と line_map に push する数が必ず1対1になること
    // （out.lines().count() == line_map.len() が保たれること）の回帰確認。
    let src = r#"OnBoot => {
    湊: A
    B
    ;
    湊: C
    let x = 1
}
OnClose => {
    湊: D
}"#;
    let result = preprocess(src).expect("preprocess failed");
    assert_eq!(result.line_map.len(), result.src.lines().count());
}

#[test]
fn test_rich_to_japanese_resolves_line_after_dialogue_merge() {
    // セリフ本文と継続行の結合でpreprocess後の行数が元ソースより
    // 減っても、rich_to_japaneseが返す行番号は元ソースの行番号と
    // 一致すること（実機で3行分ズレるバグが発生していた）。
    let src = r#"OnBoot => {
    湊: ほげ
    ふが
    let x
}"#;
    // 元ソースでは「let x」は4行目。
    let pre = preprocess(src).expect("preprocess failed");
    let errors = program_with_include()
        .parse(&*pre.src)
        .into_result()
        .expect_err("「let x」は「=値」を欠いており構文エラーになるはず");
    let msg = rich_to_japanese(&errors[0], &pre, "main.mnt");
    assert!(
        msg.contains("4行目"),
        "preprocess後にズレた行番号ではなく元ソースの4行目が報告されるべき: {}",
        msg
    );
}
