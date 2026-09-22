//! Operation-level undo history for the markup editor.
//!
//! Commits, deletes and moves are all undoable. The editor keeps two
//! stacks (`undos`, `redos`); a fresh edit clears redo. These functions
//! are the pure core so the contract is testable without a Window.

/// One undoable operation. `Move` keeps the full before/after values so
/// a restore is a single assignment.
#[derive(Clone)]
pub enum Edit<T> {
    /// A value appended to the list.
    Add(T),
    /// The value removed from an index.
    Remove(usize, T),
    /// The value at an index, before and after the change.
    Move(usize, T, T),
}

/// Apply an edit to the list (redo direction).
pub fn apply_forward<T: Clone>(items: &mut Vec<T>, edit: &Edit<T>) {
    match edit {
        Edit::Add(v) => items.push(v.clone()),
        Edit::Remove(i, _) => {
            if *i < items.len() {
                items.remove(*i);
            }
        }
        Edit::Move(i, _, new) => {
            if *i < items.len() {
                items[*i] = new.clone();
            }
        }
    }
}

/// Reverse an edit on the list (undo direction).
pub fn apply_inverse<T: Clone>(items: &mut Vec<T>, edit: &Edit<T>) {
    match edit {
        Edit::Add(_) => {
            items.pop();
        }
        Edit::Remove(i, v) => {
            items.insert((*i).min(items.len()), v.clone());
        }
        Edit::Move(i, old, _) => {
            if *i < items.len() {
                items[*i] = old.clone();
            }
        }
    }
}

// WHY: the class closed here is "undo pops the wrong thing": a delete
// or move that bypassed the history stack made ctrl+z revert an
// unrelated older edit. Every Edit variant must round-trip exactly.
// Composite rasterization is covered by the on-rig QA scripts instead.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_round_trips() {
        // The recorded value must match the content at the index, as
        // the editor records it when the edit happens.
        for edit in [Edit::Add(10), Edit::Remove(0, 1), Edit::Move(0, 1, 31)] {
            let mut items = vec![1, 2, 3];
            let before = items.clone();
            apply_forward(&mut items, &edit);
            apply_inverse(&mut items, &edit);
            assert_eq!(items, before);
        }
    }

    #[test]
    fn remove_then_add_undoes_in_reverse_order() {
        let mut items = vec![0, 1, 2];
        let mut undos = Vec::new();
        let removed = items.remove(1);
        undos.push(Edit::Remove(1, removed));
        items.push(9);
        undos.push(Edit::Add(9));
        for edit in undos.iter().rev() {
            apply_inverse(&mut items, edit);
        }
        assert_eq!(items, vec![0, 1, 2]);
    }

    #[test]
    fn move_stores_before_and_after() {
        let mut items = vec![1];
        let edit = Edit::Move(0, 1, 50);
        apply_forward(&mut items, &edit);
        assert_eq!(items, vec![50]);
        apply_inverse(&mut items, &edit);
        assert_eq!(items, vec![1]);
    }

    #[test]
    fn out_of_range_index_is_a_no_op() {
        let mut items = vec![0];
        apply_forward(&mut items, &Edit::Remove(5, 1));
        assert_eq!(items, vec![0]);
        apply_forward(&mut items, &Edit::Move(5, 1, 2));
        assert_eq!(items, vec![0]);
        apply_inverse(&mut items, &Edit::Move(5, 1, 2));
        assert_eq!(items, vec![0]);
        // Remove inverse clamps to the end instead of panicking.
        apply_inverse(&mut items, &Edit::Remove(5, 7));
        assert_eq!(items, vec![0, 7]);
    }
}
