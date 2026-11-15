//! Who meets whom.
//!
//! Every waiting person is a point, every allowed pair an edge with a weight. A round picks the set
//! of pairs with the greatest total weight in which nobody appears twice: maximum-weight matching
//! on a general graph. Up to 18 people it is solved exactly, by dynamic programming over subsets.
//! Above that, a greedy start is repaired by local moves, and then small neighbourhoods of the
//! group are taken out one at a time and solved exactly again (large neighbourhood search), which
//! can only ever make the total better.

/// Largest group solved exactly. 2^18 states of nine bytes each is about 2.4 MB and a few
/// milliseconds.
pub const EXACT_LIMIT: usize = 18;

/// Size of a neighbourhood solved exactly while improving a large group.
const WINDOW: usize = 14;

/// Every pair counts for this much before its own score, so a round always prefers pairing more
/// people over pairing fewer people slightly better.
pub const PAIR_BONUS: i64 = 1_000_000;

/// Weights for `n` people as a flat `n * n` table. `None` means the two must not meet.
pub struct Weights {
    n: usize,
    w: Vec<Option<i64>>,
}

impl Weights {
    pub fn new(n: usize) -> Self {
        Self {
            n,
            w: vec![None; n * n],
        }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    pub fn set(&mut self, a: usize, b: usize, weight: i64) {
        if a != b {
            self.w[a * self.n + b] = Some(weight);
            self.w[b * self.n + a] = Some(weight);
        }
    }

    #[inline]
    pub fn get(&self, a: usize, b: usize) -> Option<i64> {
        self.w[a * self.n + b]
    }

    pub fn total(&self, pairs: &[(usize, usize)]) -> i64 {
        pairs
            .iter()
            .map(|&(a, b)| self.get(a, b).expect("a chosen pair must be allowed"))
            .sum()
    }
}

/// The best set of pairs. Each pair is `(low, high)` and the list is sorted, so the same input
/// always gives the same answer.
pub fn solve(weights: &Weights) -> Vec<(usize, usize)> {
    let mut pairs = if weights.len() <= EXACT_LIMIT {
        exact(weights)
    } else {
        improved(weights)
    };
    for p in &mut pairs {
        if p.0 > p.1 {
            *p = (p.1, p.0);
        }
    }
    pairs.sort_unstable();
    pairs
}

/// `best[set]` is the greatest weight reachable using only the people in `set`. The lowest person
/// in the set either sits this round out or takes one partner from the rest.
fn exact(weights: &Weights) -> Vec<(usize, usize)> {
    let n = weights.len();
    if n < 2 {
        return Vec::new();
    }
    let full = 1usize << n;
    let mut best = vec![0i64; full];
    let mut partner = vec![u8::MAX; full];
    for set in 1..full {
        let i = set.trailing_zeros() as usize;
        let rest = set & (set - 1);
        let (mut top, mut with) = (best[rest], u8::MAX);
        let mut others = rest;
        while others != 0 {
            let j = others.trailing_zeros() as usize;
            others &= others - 1;
            if let Some(w) = weights.get(i, j) {
                let value = w + best[rest & !(1 << j)];
                if value > top {
                    top = value;
                    with = j as u8;
                }
            }
        }
        best[set] = top;
        partner[set] = with;
    }
    let mut pairs = Vec::with_capacity(n / 2);
    let mut set = full - 1;
    while set != 0 {
        let i = set.trailing_zeros() as usize;
        set &= set - 1;
        let j = partner[set | (1 << i)];
        if j != u8::MAX {
            pairs.push((i, j as usize));
            set &= !(1usize << j);
        }
    }
    pairs
}

/// Heaviest edge first, then three kinds of repair until nothing improves: re-pairing two pairs,
/// joining two people left out, and letting someone left out take a partner whose old partner can
/// be placed again or is worth less.
#[allow(clippy::needless_range_loop)] // the loops index several tables by person at once
fn improved(weights: &Weights) -> Vec<(usize, usize)> {
    const FREE: usize = usize::MAX;
    let n = weights.len();
    let mut edges: Vec<(i64, usize, usize)> = Vec::new();
    for a in 0..n {
        for b in a + 1..n {
            if let Some(w) = weights.get(a, b) {
                edges.push((w, a, b));
            }
        }
    }
    edges.sort_unstable_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
    let mut mate = vec![FREE; n];
    for &(_, a, b) in &edges {
        if mate[a] == FREE && mate[b] == FREE {
            mate[a] = b;
            mate[b] = a;
        }
    }
    let w = |a: usize, b: usize| weights.get(a, b);
    for _ in 0..64 {
        let mut changed = false;
        // Two pairs (a,b) and (c,d): try (a,c)+(b,d) and (a,d)+(b,c).
        for a in 0..n {
            let b = mate[a];
            if b == FREE || b < a {
                continue;
            }
            for c in a + 1..n {
                let d = mate[c];
                if d == FREE || d < c || c == b {
                    continue;
                }
                let (Some(now_ab), Some(now_cd)) = (w(a, mate[a]), w(c, d)) else {
                    continue;
                };
                let now = now_ab + now_cd;
                let b = mate[a];
                let swap1 = w(a, c).zip(w(b, d)).map(|(x, y)| x + y);
                let swap2 = w(a, d).zip(w(b, c)).map(|(x, y)| x + y);
                if swap1.is_some_and(|v| v > now) && swap1 >= swap2 {
                    (mate[a], mate[c], mate[b], mate[d]) = (c, a, d, b);
                    changed = true;
                } else if swap2.is_some_and(|v| v > now) {
                    (mate[a], mate[d], mate[b], mate[c]) = (d, a, c, b);
                    changed = true;
                }
                if mate[a] == FREE || mate[a] < a {
                    break;
                }
            }
        }
        // Someone left out.
        for u in 0..n {
            if mate[u] != FREE {
                continue;
            }
            // Another person left out who may meet them.
            if let Some(v) = (0..n)
                .filter(|&v| v != u && mate[v] == FREE)
                .filter_map(|v| w(u, v).map(|x| (x, v)))
                .max()
                .map(|(_, v)| v)
            {
                mate[u] = v;
                mate[v] = u;
                changed = true;
                continue;
            }
            // Take a's place beside b when that is worth more than a and b together.
            let mut gain_best = 0i64;
            let mut take: Option<(usize, usize)> = None;
            for a in 0..n {
                let b = mate[a];
                if b == FREE || a == u {
                    continue;
                }
                if let (Some(new), Some(old)) = (w(u, b), w(a, b))
                    && new - old > gain_best
                {
                    gain_best = new - old;
                    take = Some((a, b));
                }
            }
            if let Some((a, b)) = take {
                mate[a] = FREE;
                mate[u] = b;
                mate[b] = u;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Take a neighbourhood out, solve it exactly, and keep the answer only if it is better. The
    // neighbourhood always contains both halves of any pair it touches, so everyone outside it
    // keeps their partner.
    let mut state = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ edges.len() as u64;
    let mut random = move |below: usize| {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % below.max(1) as u64) as usize
    };
    let rounds = (3 * n).min(240);
    let mut idle = 0;
    for round in 0..rounds {
        let free: Vec<usize> = (0..n).filter(|&v| mate[v] == FREE).collect();
        // Half the time start from someone left out: that is where a better answer hides.
        let seed = if !free.is_empty() && round % 2 == 0 {
            free[(round / 2) % free.len()]
        } else {
            random(n)
        };
        let mut inside = vec![false; n];
        let mut window: Vec<usize> = Vec::with_capacity(WINDOW);
        let mut add = |v: usize, window: &mut Vec<usize>| {
            for x in [v, mate[v]] {
                if x != FREE && !inside[x] {
                    inside[x] = true;
                    window.push(x);
                }
            }
        };
        add(seed, &mut window);
        let mut at = 0;
        while at < window.len() && window.len() + 2 <= WINDOW {
            let u = window[at];
            at += 1;
            let mut near: Vec<usize> = (0..n).filter(|&v| weights.get(u, v).is_some()).collect();
            // A different order each time, so repeated windows around one person differ.
            for i in (1..near.len()).rev() {
                near.swap(i, random(i + 1));
            }
            // People left out first: they are the ones a new arrangement can bring in.
            near.sort_by_key(|&v| mate[v] != FREE);
            for v in near {
                if window.len() + 2 > WINDOW {
                    break;
                }
                add(v, &mut window);
            }
        }
        let k = window.len();
        let mut now = 0i64;
        let mut sub = Weights::new(k);
        for i in 0..k {
            let m = mate[window[i]];
            if m != FREE && window[i] < m {
                now += weights.get(window[i], m).unwrap_or(0);
            }
            for j in i + 1..k {
                if let Some(weight) = weights.get(window[i], window[j]) {
                    sub.set(i, j, weight);
                }
            }
        }
        let better = exact(&sub);
        if sub.total(&better) > now {
            for &v in &window {
                mate[v] = FREE;
            }
            for (i, j) in better {
                mate[window[i]] = window[j];
                mate[window[j]] = window[i];
            }
            idle = 0;
        } else {
            idle += 1;
            if idle >= 80 {
                break;
            }
        }
    }
    (0..n)
        .filter(|&a| mate[a] != FREE && a < mate[a])
        .map(|a| (a, mate[a]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator, so failures can be replayed.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn random(n: usize, rng: &mut Rng, density: u64) -> Weights {
        let mut w = Weights::new(n);
        for a in 0..n {
            for b in a + 1..n {
                if rng.below(100) < density {
                    w.set(a, b, PAIR_BONUS + rng.below(10_000) as i64);
                }
            }
        }
        w
    }

    /// Every possible matching, tried one by one. Only usable for small groups.
    fn brute(w: &Weights, free: &mut Vec<usize>) -> i64 {
        let Some(a) = free.pop() else { return 0 };
        let mut best = brute(w, free); // a sits out
        for i in 0..free.len() {
            let b = free[i];
            if let Some(weight) = w.get(a, b) {
                free.swap_remove(i);
                best = best.max(weight + brute(w, free));
                free.push(b);
                let last = free.len() - 1;
                free.swap(i, last);
            }
        }
        free.push(a);
        best
    }

    fn valid(w: &Weights, pairs: &[(usize, usize)]) {
        let mut seen = vec![false; w.len()];
        for &(a, b) in pairs {
            assert!(a < b, "pairs are ordered");
            assert!(w.get(a, b).is_some(), "only allowed pairs");
            assert!(!seen[a] && !seen[b], "nobody twice");
            seen[a] = true;
            seen[b] = true;
        }
    }

    #[test]
    fn empty_and_tiny_groups() {
        assert_eq!(solve(&Weights::new(0)), [] as [(usize, usize); 0]);
        assert_eq!(solve(&Weights::new(1)), [] as [(usize, usize); 0]);
        let mut two = Weights::new(2);
        assert!(
            solve(&two).is_empty(),
            "two people who may not meet stay apart"
        );
        two.set(0, 1, 5);
        assert_eq!(solve(&two), [(0, 1)]);
    }

    #[test]
    fn exact_agrees_with_trying_everything() {
        let mut rng = Rng(7);
        for round in 0..300 {
            let n = 2 + (round % 9);
            let density = 30 + rng.below(70);
            let w = random(n, &mut rng, density);
            let pairs = solve(&w);
            valid(&w, &pairs);
            let mut free: Vec<usize> = (0..n).collect();
            assert_eq!(w.total(&pairs), brute(&w, &mut free), "n={n} round={round}");
        }
    }

    #[test]
    fn more_pairs_beat_better_pairs() {
        // 0-1 is a wonderful pair, but then 2 and 3 cannot meet. 0-2 and 1-3 pair everyone.
        let mut w = Weights::new(4);
        w.set(0, 1, PAIR_BONUS + 9_999);
        w.set(0, 2, PAIR_BONUS + 1);
        w.set(1, 3, PAIR_BONUS + 1);
        assert_eq!(solve(&w), [(0, 2), (1, 3)]);
    }

    #[test]
    fn with_an_odd_number_the_cheapest_person_waits() {
        let mut w = Weights::new(3);
        w.set(0, 1, PAIR_BONUS + 10);
        w.set(1, 2, PAIR_BONUS + 500);
        w.set(0, 2, PAIR_BONUS + 20);
        assert_eq!(solve(&w), [(1, 2)]);
    }

    #[test]
    fn the_same_input_gives_the_same_answer() {
        let mut rng = Rng(99);
        let w = random(14, &mut rng, 60);
        assert_eq!(solve(&w), solve(&w));
    }

    #[test]
    fn the_large_group_method_is_valid_and_close_to_exact() {
        let mut rng = Rng(2026);
        let mut worst = 1.0f64;
        for _ in 0..60 {
            let n = 10 + rng.below(9) as usize; // small enough to know the true answer
            let density = 25 + rng.below(75);
            let w = random(n, &mut rng, density);
            let mut heuristic = improved(&w);
            for p in &mut heuristic {
                if p.0 > p.1 {
                    *p = (p.1, p.0);
                }
            }
            valid(&w, &heuristic);
            let best = w.total(&exact(&w));
            if best > 0 {
                worst = worst.min(w.total(&heuristic) as f64 / best as f64);
            }
        }
        assert!(worst >= 0.97, "worst case was {worst:.3} of the true best");
    }

    #[test]
    fn two_hundred_people_are_paired_quickly() {
        let mut rng = Rng(5);
        let w = random(200, &mut rng, 80);
        let started = std::time::Instant::now();
        let pairs = solve(&w);
        valid(&w, &pairs);
        assert_eq!(pairs.len(), 100, "a dense group of 200 pairs everyone");
        assert!(
            started.elapsed().as_millis() < 6_000,
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_sparse_large_group_still_pairs_almost_everyone() {
        // A ring of 60 people where each may only meet their two neighbours: a perfect pairing
        // exists, and a greedy start alone usually misses it.
        let mut rng = Rng(60);
        let mut w = Weights::new(60);
        for a in 0..60 {
            w.set(a, (a + 1) % 60, PAIR_BONUS + rng.below(10_000) as i64);
        }
        let pairs = solve(&w);
        valid(&w, &pairs);
        assert!(pairs.len() >= 29, "only {} of 30 pairs", pairs.len());
    }

    #[test]
    fn eighteen_people_are_solved_exactly_in_good_time() {
        let mut rng = Rng(18);
        let w = random(18, &mut rng, 100);
        let started = std::time::Instant::now();
        let pairs = solve(&w);
        assert_eq!(pairs.len(), 9);
        assert!(
            started.elapsed().as_millis() < 3_000,
            "took {:?}",
            started.elapsed()
        );
    }
}
