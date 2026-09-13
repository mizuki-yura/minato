// runtime.rs
// トーク選択ロジック（重複回避・1周保証）

use std::collections::HashMap;
use crate::parser::Talk;

pub struct TalkSelector {
    queue: HashMap<String, Vec<usize>>,
    /// 直近にqueueを組み直したときのalive集合。condの変化検知に使う。
    /// queue自体は消費されて減っていくため、queueの中身とaliveを
    /// 直接比較すると「1周の途中で残り枚数が減っただけ」を
    /// 「候補が変わった」と誤判定してしまう。そのため別途保持する。
    built_from: HashMap<String, Vec<usize>>,
    last: HashMap<String, usize>,
}
impl TalkSelector {
    pub fn new() -> Self {
        Self { queue: HashMap::new(), built_from: HashMap::new(), last: HashMap::new() }
    }

    /// `alive`から1件選んで返す。呼び出し元は現状すべて事前に
    /// `alive.is_empty()`を弾いているが、その契約は型で保証されておらず
    /// 将来別の呼び出し元が追加されると崩れうる。この関数自身が空でも
    /// panicせず`None`を返すことで、その契約が破られても
    /// プロセスクラッシュに直結しないようにする。
    pub fn select_alive<'a>(
        &mut self,
        event: &str,
        alive: &[(usize, &'a Talk)]
    ) -> Option<&'a Talk> {
        if alive.is_empty() {
            return None;
        }
        if alive.len() == 1 {
            self.last.insert(event.to_string(), alive[0].0);
            // 候補が1件になった時点で「1周保証」も「隣接非重複」も
            // 定義できない状態に入っている。残っているqueueは
            // もう成り立たない前提のもとで組まれた配列なので捨てる。
            // built_fromも一緒に消さないと、候補が元の集合に戻ったとき
            // 「変化なし」と誤判定されてrebuildされず、
            // ここでlastに記録したIDが直後にpopされうる。
            self.queue.remove(event);
            self.built_from.remove(event);
            return Some(alive[0].1);
        }

        let alive_ids: Vec<usize> = alive.iter().map(|(o, _)| *o).collect();

        let needs_rebuild = match (self.queue.get(event), self.built_from.get(event)) {
            (Some(q), Some(built)) => q.is_empty() || !same_id_set(built, &alive_ids),
            _ => true,
        };

        if needs_rebuild {
            let forbid_first = self.last.get(event).copied();
            self.queue.insert(event.to_string(), shuffled(&alive_ids, forbid_first));
            self.built_from.insert(event.to_string(), alive_ids);
        }

        // rebuild直後ならqueueは非空のはずだが、それも呼び出し元の契約と
        // 同様に型で保証されているわけではないため、`?`で素直にNoneへ
        // 逃がす（unwrap/expectでのpanicを避ける）。
        let q = self.queue.get_mut(event)?;
        let picked_orig = q.pop()?;
        self.last.insert(event.to_string(), picked_orig);
        alive.iter().find(|(o, _)| *o == picked_orig).map(|(_, t)| *t)
    }
}

/// 2つのID列が同じ要素集合かどうか（順序は問わない）
fn same_id_set(a: &[usize], b: &[usize]) -> bool {
    if a.len() != b.len() { return false; }
    let mut a_sorted = a.to_vec();
    let mut b_sorted = b.to_vec();
    a_sorted.sort_unstable();
    b_sorted.sort_unstable();
    a_sorted == b_sorted
}

/// idsをFisher-Yatesでシャッフルし、末尾（=最初に配られる要素）が
/// forbid_firstと被らないようにする。
/// popで先頭から配りたいので、シャッフル後さらに末尾を調整する形をとる。
fn shuffled(ids: &[usize], forbid_first: Option<usize>) -> Vec<usize> {
    let mut v = ids.to_vec();
    let n = v.len();
    if n == 0 {
        // 呼び出し元(select_alive)は現状alive非空を保証してから呼ぶが、
        // 将来の変更でここが崩れても v[n-1] のアンダーフローで
        // panicしないよう自己防御する。
        return v;
    }
    for i in (1..n).rev() {
        let j = (crate::next_rand() as usize) % (i + 1);
        v.swap(i, j);
    }
    // 末尾（次にpopされる要素）がforbid_firstと同じなら、
    // 他の位置とswapして回避する。ids.len() >= 2 が呼び出し元で保証されている。
    if let Some(forbid) = forbid_first {
        if v[n - 1] == forbid {
            let swap_with = (0..n - 1).find(|&i| v[i] != forbid).unwrap_or(0);
            v.swap(n - 1, swap_with);
        }
    }
    v
}

/// 簡易乱数（外部クレート不要）
pub fn simple_rand() -> u32 {
    crate::next_rand()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_talk(event: &str) -> Talk {
        Talk { event: event.to_string(), cond: None, body: vec![] }
    }

    fn as_alive(talks: &[Talk]) -> Vec<(usize, &Talk)> {
        talks.iter().enumerate().collect()
    }

    #[test]
    fn test_empty_alive_returns_none_instead_of_panicking() {
        // 呼び出し元の「aliveは非空」という契約が将来崩れても、
        // select_alive自身がpanicせずNoneを返すことの確認
        let mut sel = TalkSelector::new();
        let alive: Vec<(usize, &Talk)> = vec![];
        assert!(sel.select_alive("OnRandomTalk", &alive).is_none());
    }

    #[test]
    fn test_shuffled_with_empty_ids_does_not_panic() {
        // v[n-1]のアンダーフローを避けられていることの直接確認
        let result = shuffled(&[], None);
        assert!(result.is_empty());
        let result = shuffled(&[], Some(42));
        assert!(result.is_empty());
    }

    #[test]
    fn test_single_candidate() {
        let candidates = vec![dummy_talk("OnBoot")];
        let alive = as_alive(&candidates);
        let mut sel = TalkSelector::new();
        for _ in 0..5 {
            let t = sel.select_alive("OnBoot", &alive).expect("aliveは非空");
            assert_eq!(t.event, "OnBoot");
        }
    }

    #[test]
    fn test_never_repeats_immediately() {
        let candidates: Vec<Talk> = (0..5).map(|_| dummy_talk("OnRandomTalk")).collect();
        let alive: Vec<(usize, &Talk)> = as_alive(&candidates);
        let mut sel = TalkSelector::new();

        let mut prev: Option<usize> = None;
        for _ in 0..500 {
            let picked = sel.select_alive("OnRandomTalk", &alive).expect("aliveは非空");
            let idx = candidates.iter().position(|t| std::ptr::eq(t, picked)).unwrap();
            assert_ne!(Some(idx), prev, "同じトークが連続で選ばれた");
            prev = Some(idx);
        }
    }


        #[test]
fn test_single_candidate_detour_does_not_break_adjacent_rule() {
    // 候補4件 → condで一時的に1件 → 4件に復帰、という経路で
    // 「1件だった時に選ばれたトーク」が直後に再び選ばれないこと。
    // 修正前は built_from が復帰後の集合と一致するため rebuild されず、
    // 残っていた queue から同じIDがpopされて隣接重複が起こりえた。
    let candidates: Vec<Talk> = (0..4).map(|_| dummy_talk("OnRandomTalk")).collect();
    let alive_full: Vec<(usize, &Talk)> = as_alive(&candidates);
    let alive_single: Vec<(usize, &Talk)> = candidates.iter().enumerate().take(1).collect();

    for trial in 0..200 {
        let mut sel = TalkSelector::new();

        // queueを途中まで消費させて、残りが残っている状態を作る
        sel.select_alive("OnRandomTalk", &alive_full);
        sel.select_alive("OnRandomTalk", &alive_full);

        // condで1件だけに絞られる（早期returnを通る）
        let single = sel.select_alive("OnRandomTalk", &alive_single).expect("aliveは非空");
        let single_idx = candidates.iter().position(|t| std::ptr::eq(t, single)).unwrap();

        // condが戻って4件に復帰
        let next = sel.select_alive("OnRandomTalk", &alive_full).expect("aliveは非空");
        let next_idx = candidates.iter().position(|t| std::ptr::eq(t, next)).unwrap();

        assert_ne!(
            single_idx, next_idx,
            "試行{}: 1件経由の直後に同じトークが選ばれた", trial
        );
    }
}

    #[test]
    fn test_all_candidates_appear_exactly_once_per_cycle() {
        // 1周（候補数ぶん）引くと、全候補がちょうど1回ずつ出る
        let candidates: Vec<Talk> = (0..5).map(|_| dummy_talk("OnRandomTalk")).collect();
        let alive: Vec<(usize, &Talk)> = as_alive(&candidates);
        let mut sel = TalkSelector::new();

        for cycle in 0..20 {
            let mut seen = vec![0usize; candidates.len()];
            for _ in 0..5 {
                let picked = sel.select_alive("OnRandomTalk", &alive).expect("aliveは非空");
                let idx = candidates.iter().position(|t| std::ptr::eq(t, picked)).unwrap();
                seen[idx] += 1;
            }
            assert_eq!(seen, vec![1, 1, 1, 1, 1], "周回{}で重複または漏れがある: {:?}", cycle, seen);
        }
    }

    #[test]
    fn test_no_adjacent_repeat_across_many_cycles() {
        // 奇数個の候補でも周回の境目で連続しないことを確認
        let candidates: Vec<Talk> = (0..3).map(|_| dummy_talk("OnRandomTalk")).collect();
        let alive: Vec<(usize, &Talk)> = as_alive(&candidates);
        let mut sel = TalkSelector::new();

        let mut prev: Option<usize> = None;
        for _ in 0..300 {
            let picked = sel.select_alive("OnRandomTalk", &alive).expect("aliveは非空");
            let idx = candidates.iter().position(|t| std::ptr::eq(t, picked)).unwrap();
            assert_ne!(Some(idx), prev, "周回境界で同じトークが連続した");
            prev = Some(idx);
        }
    }

    #[test]
    fn test_cond_change_rebuilds_queue_safely() {
        // condで候補が減った場合、queueに存在しない候補が残っていても
        // panicせず正しく組み直されることの確認
        let candidates: Vec<Talk> = (0..4).map(|_| dummy_talk("OnRandomTalk")).collect();
        let alive_full: Vec<(usize, &Talk)> = as_alive(&candidates);
        let alive_partial: Vec<(usize, &Talk)> = candidates.iter().enumerate().take(2).collect();
        let mut sel = TalkSelector::new();

        for _ in 0..20 {
            sel.select_alive("OnRandomTalk", &alive_full);
        }
        // 候補が4→2に減った状態で呼んでもpanicしないこと
        for _ in 0..20 {
            let picked = sel.select_alive("OnRandomTalk", &alive_partial).expect("aliveは非空");
            assert!(candidates[0..2].iter().any(|t| std::ptr::eq(t, picked)));
        }
    }

    
#[test]
fn test_queue_not_rebuilt_mid_cycle_with_stable_candidates() {
    // 候補が変わらない限り、1周の途中で毎回組み直されていないことを
    // 間接的に確認する。組み直されていれば全候補一致は保証されない。
    let candidates: Vec<Talk> = (0..5).map(|_| dummy_talk("OnRandomTalk")).collect();
    let alive: Vec<(usize, &Talk)> = as_alive(&candidates);
    let mut sel = TalkSelector::new();

    for cycle in 0..50 {
        let mut seen = vec![0usize; candidates.len()];
        for _ in 0..5 {
            let picked = sel.select_alive("OnRandomTalk", &alive).expect("aliveは非空");
            let idx = candidates.iter().position(|t| std::ptr::eq(t, picked)).unwrap();
            seen[idx] += 1;
        }
        assert_eq!(seen, vec![1, 1, 1, 1, 1], "周回{}で重複または漏れがある: {:?}", cycle, seen);
    }
}
}