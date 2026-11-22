//! Half-open `[start, end)` millisecond intervals.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: i64,
    pub end: i64,
}

/// Sorted, merged, non-empty spans. Touching spans (`[0,5)`, `[5,9)`) merge.
pub fn union(spans: &[Span]) -> Vec<Span> {
    let mut v: Vec<Span> = spans.iter().copied().filter(|s| s.end > s.start).collect();
    v.sort_by_key(|s| s.start);
    let mut out: Vec<Span> = Vec::with_capacity(v.len());
    for s in v {
        match out.last_mut() {
            Some(l) if s.start <= l.end => l.end = l.end.max(s.end),
            _ => out.push(s),
        }
    }
    out
}

/// Parts of `a` not covered by `b`.
pub fn subtract(a: &[Span], b: &[Span]) -> Vec<Span> {
    let (a, b) = (union(a), union(b));
    let mut out = Vec::new();
    let mut j = 0;
    for s in a {
        let mut cur = s.start;
        // b spans ending before this span are irrelevant to it and to all later (sorted) spans
        while j < b.len() && b[j].end <= s.start {
            j += 1;
        }
        let mut k = j;
        while k < b.len() && b[k].start < s.end {
            if b[k].start > cur {
                out.push(Span { start: cur, end: b[k].start });
            }
            cur = cur.max(b[k].end);
            k += 1;
        }
        if cur < s.end {
            out.push(Span { start: cur, end: s.end });
        }
    }
    out
}

/// Intersect each span with `window`; empty results dropped.
pub fn clip(spans: &[Span], window: Span) -> Vec<Span> {
    union(spans)
        .into_iter()
        .map(|s| Span { start: s.start.max(window.start), end: s.end.min(window.end) })
        .filter(|s| s.end > s.start)
        .collect()
}

pub fn total_ms(spans: &[Span]) -> i64 {
    spans.iter().map(|s| s.end - s.start).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(a: i64, b: i64) -> Span {
        Span { start: a, end: b }
    }

    #[test]
    fn union_merges_overlaps() {
        assert_eq!(union(&[s(0, 10), s(5, 20), s(30, 40)]), vec![s(0, 20), s(30, 40)]);
    }
    #[test]
    fn subtract_handles_partial_and_full_cover() {
        assert_eq!(subtract(&[s(0, 100)], &[s(10, 20), s(50, 200)]), vec![s(0, 10), s(20, 50)]);
    }
    #[test]
    fn overlapping_subtractions_not_double_counted() {
        let removed = union(&[s(10, 60), s(40, 90)]);
        assert_eq!(total_ms(&subtract(&[s(0, 100)], &removed)), 20);
    }
    #[test]
    fn clip_to_window() {
        assert_eq!(clip(&[s(-50, 30), s(80, 500)], s(0, 100)), vec![s(0, 30), s(80, 100)]);
    }
    #[test]
    fn empty_and_unsorted_inputs() {
        assert_eq!(union(&[]), vec![]);
        assert_eq!(union(&[s(5, 5), s(9, 3)]), vec![]);
        assert_eq!(subtract(&[s(50, 100), s(0, 60)], &[]), vec![s(0, 100)]);
    }
}
