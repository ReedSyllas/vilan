pub fn plural(n: usize, singular: &str, plural: &str) -> String {
    if n == 1 { singular } else { plural }.to_string()
}

use std::cell::Cell;

thread_local! {
    static RECURSION_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// A safety net for the recursive type operations (`reconcile_type`,
/// `substitute_type`, the transformer's `resolve_type_id`). A self-mapping or
/// otherwise pathological generic graph that slips past the explicit guards must
/// degrade to a graceful bail rather than overflow the stack — a compiler should
/// never crash on user input. The limit is far above any real type's nesting.
pub struct RecursionGuard;

impl RecursionGuard {
    /// Enters one level of recursion; `None` once the depth limit is reached, so
    /// the caller can return a graceful fallback instead of recursing.
    pub fn enter() -> Option<RecursionGuard> {
        RECURSION_DEPTH.with(|depth| {
            let current = depth.get();
            if current >= 2048 {
                None
            } else {
                depth.set(current + 1);
                Some(RecursionGuard)
            }
        })
    }
}

impl Drop for RecursionGuard {
    fn drop(&mut self) {
        RECURSION_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}
