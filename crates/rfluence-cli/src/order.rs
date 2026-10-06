//! Putting a parent's pages and folders in the config's order with as few moves as possible.
//! See design.md, "Child page order".

use std::collections::HashSet;

/// A move: put `id` right before or after `target`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub id: String,
    pub after: bool,
    pub target: String,
}

/// The moves that put `desired` (the tree's children of a parent, in order) in that order.
///
/// `current` is the parent's children as they are now, in order (it may hold pages that
/// aren't in the tree: they're never moved). `new` are the children created by this upload:
/// they're always put in place. Existing children are only moved if `move_existing`: the
/// longest run of them already in the right relative order stays, and the rest move.
/// Returns the moves, and the existing children that are out of order but weren't moved.
pub fn moves(desired: &[String], current: &[String], new: &HashSet<String>, move_existing: bool) -> (Vec<Move>, Vec<String>) {
    let wanted: HashSet<&String> = desired.iter().collect();
    // Our children as they are now.
    let mut order: Vec<String> = current.iter().filter(|id| wanted.contains(id)).cloned().collect();
    let rank = |id: &String| desired.iter().position(|d| d == id).expect("filtered to desired");
    // Existing children: keep the longest run in the right order.
    let existing: Vec<&String> = order.iter().filter(|id| !new.contains(*id)).collect();
    let keep: HashSet<String> = longest_increasing(&existing.iter().map(|id| rank(id)).collect::<Vec<_>>())
        .into_iter()
        .map(|i| existing[i].clone())
        .collect();
    let out_of_order: Vec<String> = existing.iter().filter(|id| !keep.contains(**id)).map(|id| (*id).clone()).collect();
    let mut moving: HashSet<&String> = new.iter().collect();
    if move_existing {
        moving.extend(out_of_order.iter());
    }

    let mut moves = Vec::new();
    for (i, id) in desired.iter().enumerate() {
        if !moving.contains(id) {
            continue;
        }
        let Some(pos) = order.iter().position(|o| o == id) else { continue };
        let mv = match i.checked_sub(1).map(|p| &desired[p]) {
            // After its predecessor (which is in place: earlier in the desired order).
            Some(prev) if order.contains(prev) => {
                if pos > 0 && &order[pos - 1] == prev {
                    continue;
                }
                Move { id: id.clone(), after: true, target: prev.clone() }
            }
            // The first (or its predecessor isn't under this parent): before everything else.
            _ => {
                let first_other = order.iter().find(|o| *o != id).cloned();
                match first_other {
                    Some(target) if pos != 0 => Move { id: id.clone(), after: false, target },
                    _ => continue,
                }
            }
        };
        order.remove(pos);
        let at = order.iter().position(|o| *o == mv.target).expect("target is in order");
        order.insert(if mv.after { at + 1 } else { at }, id.clone());
        moves.push(mv);
    }
    let unmoved = if move_existing { Vec::new() } else { out_of_order };
    (moves, unmoved)
}

/// The indexes of a longest strictly increasing subsequence.
fn longest_increasing(values: &[usize]) -> Vec<usize> {
    let n = values.len();
    let mut length = vec![1; n];
    let mut previous = vec![usize::MAX; n];
    for i in 0..n {
        for j in 0..i {
            if values[j] < values[i] && length[j] + 1 > length[i] {
                length[i] = length[j] + 1;
                previous[i] = j;
            }
        }
    }
    let Some(mut i) = (0..n).max_by_key(|&i| (length[i], usize::MAX - i)) else { return Vec::new() };
    let mut out = vec![i];
    while previous[i] != usize::MAX {
        i = previous[i];
        out.push(i);
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    /// Apply moves to a list, as Confluence would.
    fn apply(current: &[String], moves: &[Move]) -> Vec<String> {
        let mut order = current.to_vec();
        for m in moves {
            order.retain(|o| *o != m.id);
            let at = order.iter().position(|o| *o == m.target).unwrap();
            order.insert(if m.after { at + 1 } else { at }, m.id.clone());
        }
        order
    }

    fn check(desired: &str, current: &str, new: &str, move_existing: bool) -> (Vec<Move>, Vec<String>, Vec<String>) {
        let (desired, current) = (ids(desired), ids(current));
        let new: HashSet<String> = ids(new).into_iter().collect();
        let (moves, unmoved) = moves(&desired, &current, &new, move_existing);
        let result = apply(&current, &moves);
        (moves, unmoved, result)
    }

    #[test]
    fn already_in_order_needs_no_moves() {
        let (moves, unmoved, _) = check("a b c", "a b c", "", true);
        assert!(moves.is_empty() && unmoved.is_empty());
        // Pages people added (x, y) don't matter.
        let (moves, _, _) = check("a b c", "x a y b c", "", true);
        assert!(moves.is_empty());
    }

    #[test]
    fn places_new_children() {
        // Created pages are appended; each goes after its predecessor.
        let (moves, _, result) = check("a n1 b n2", "a b n1 n2", "n1 n2", false);
        assert_eq!(result, ids("a n1 b n2"));
        assert_eq!(moves.len(), 1, "n2 is already right after b: {moves:?}");
        // A new first child goes before everything.
        let (_, _, result) = check("n a b", "a b n", "n", false);
        assert_eq!(result, ids("n a b"));
        // A new last child appended at the end is already in place.
        let (moves, _, _) = check("a b n", "a b n", "n", false);
        assert!(moves.is_empty());
    }

    #[test]
    fn moves_existing_children_only_when_asked() {
        let (moves, unmoved, result) = check("a b c d", "a c b d", "", false);
        assert!(moves.is_empty());
        assert_eq!(unmoved.len(), 1, "one of b or c is out of order: {unmoved:?}");
        assert_eq!(result, ids("a c b d"));
        let (moves, unmoved, result) = check("a b c d", "a c b d", "", true);
        assert_eq!((moves.len(), unmoved.len()), (1, 0));
        assert_eq!(result, ids("a b c d"));
    }

    #[test]
    fn keeps_the_longest_run_in_order() {
        // Reversed: only one can stay; everything else moves, each after its predecessor.
        let (moves, _, result) = check("a b c d e", "e d c b a", "", true);
        assert_eq!(result, ids("a b c d e"));
        assert_eq!(moves.len(), 4);
        // One page moved to the front in Confluence: one move puts it back.
        let (moves, _, result) = check("a b c d e", "e a b c d", "", true);
        assert_eq!(result, ids("a b c d e"));
        assert_eq!(moves.len(), 1);
    }

    #[test]
    fn ignores_children_not_under_the_parent() {
        // b was left somewhere else (misplaced): the rest are still ordered.
        let (_, _, result) = check("a b c", "c a", "", true);
        assert_eq!(result, ids("a c"));
    }

    #[test]
    fn finds_longest_increasing_runs() {
        assert_eq!(longest_increasing(&[0, 2, 1, 3]).len(), 3);
        assert_eq!(longest_increasing(&[4, 3, 2, 1]).len(), 1);
        assert!(longest_increasing(&[]).is_empty());
    }
}
