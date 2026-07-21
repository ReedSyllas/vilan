//! Per-site leaked-byte counters (analysis-reuse.md E3 Phase 1).
//!
//! The compiler leaks a handful of `&'static` allocations per analysis by
//! design: the source text and AST arenas the `Program` borrows for `'static`
//! outlive the analysis on purpose. Backlog E3 asks that these leaks be
//! *measured*, not RSS-inferred — RSS is dominated by allocator retention from
//! rebuilding and dropping the reachable `Program` every call, which swamps the
//! few KiB of genuine per-analysis leak and is far too noisy to gate on.
//!
//! Every `Box::leak`/`String::leak` site in `vilan-core` and `vilan-lsp` calls
//! [`record`] with a [`LeakSite`] tag and the byte count it just made immortal.
//! A test reads a site's total with [`bytes`], the sum with [`total`], and
//! zeroes them between measurements with [`reset`].
//!
//! **Thread-local, not process-global.** Analysis runs inline on one thread
//! (the LSP and the leak harness each spawn a big-stack thread and run the
//! whole pipeline on it), so a thread-local counter tallies exactly the leaks
//! of the analyses that ran on the measuring thread — immune to the parallel
//! test runner, where a process-global counter's before/after deltas are
//! famously flaky (the E12 pointer-identity lesson). The cost is one `Cell` add
//! per leak, noise next to the heap allocation being leaked, so the tally stays
//! always-on rather than behind `cfg(test)` — which would not survive
//! `vilan-core` being built as a (non-test) dependency of `vilan-lsp`'s test
//! binary in any case.
//!
//! Text-site counts are exact byte lengths. AST-site counts are the shallow
//! `size_of_val` of the leaked box (a deterministic per-analysis constant): the
//! growth assertions care whether a site's contribution *plateaus*, not the
//! deep retained tree size, so the shallow figure is the right cheap signal.

use std::cell::Cell;

/// A named `Box::leak`/`String::leak` site. Discriminants index the counters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LeakSite {
    /// The LSP entry source text, leaked so the `Program` can borrow it.
    LspEntryText,
    /// The entry file's parsed AST, leaked in `analyze_source`.
    EntryAst,
    /// `parse_clean_cached`'s leaked source (content-keyed: one per content).
    ParseCleanCacheText,
    /// `parse_clean_cached`'s leaked AST (content-keyed: one per content).
    ParseCleanCacheAst,
    /// A `macro { .. }` block's synthetic entry name.
    MacroBlockEntryName,
    /// A dependency package's display name.
    DisplayName,
    /// A non-clean module's leaked source in the loader's error path.
    ModuleErrorText,
    /// A non-clean module's leaked AST/error slice in the loader's error path.
    ModuleErrorAst,
    /// The macro world's ambient-prelude import text.
    MacroPreludeText,
    /// A macro world's blanked entry source (content-keyed by `WORLDS`).
    MacroWorldText,
    /// A compiled macro world's `Program` (content-keyed by `WORLDS`).
    MacroWorldProgram,
    /// A macro's raw expansion text (content-keyed by `cached_run`).
    MacroExpansion,
    /// `parse_generated`'s leaked copy of the source it parses.
    MacroParseText,
    /// `parse_generated`'s leaked AST.
    MacroParseAst,
}

/// The number of [`LeakSite`] variants — keep in step with the enum.
const SITE_COUNT: usize = 14;

thread_local! {
    static COUNTERS: [Cell<usize>; SITE_COUNT] = const { [const { Cell::new(0) }; SITE_COUNT] };
}

/// Records `bytes` newly leaked at `site` on the current thread.
#[inline]
pub fn record(site: LeakSite, bytes: usize) {
    COUNTERS.with(|counters| {
        let cell = &counters[site as usize];
        cell.set(cell.get() + bytes);
    });
}

/// The bytes leaked at `site` on the current thread since the last [`reset`].
pub fn bytes(site: LeakSite) -> usize {
    COUNTERS.with(|counters| counters[site as usize].get())
}

/// The total bytes leaked across every site on the current thread.
pub fn total() -> usize {
    COUNTERS.with(|counters| counters.iter().map(Cell::get).sum())
}

/// Zeroes every counter on the current thread (call between measurements).
pub fn reset() {
    COUNTERS.with(|counters| {
        for cell in counters {
            cell.set(0);
        }
    });
}
