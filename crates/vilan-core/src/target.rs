//! The compilation target — which host the emitted JavaScript runs on. It selects
//! which of a library's target *layers* are reachable (a browser build can't load
//! `std::http`, which lives in `std`'s `node` overlay; a Node build can't load the
//! `browser` overlay), and how host bindings are emitted (Node's `process.exit` vs
//! none in the browser).

/// Where a compiled program runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    /// Node.js — the default. Has `process`, `node:` host modules, and stdin.
    #[default]
    Node,
    /// The browser. Has the DOM and `fetch` as globals; no `process`, no `node:`
    /// imports, no stdin.
    Browser,
    /// No host — a pure library. Only the universal `Core` platform layer is in
    /// scope (no `Node` or `Browser` std), so it can be type-checked anywhere but
    /// can't be built: emitting picks a concrete host (`--target node`/`browser`).
    None,
}

impl Target {
    /// Parses a `--target` value (`node` / `browser` / `none`), or `None` if
    /// unrecognized.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "node" => Some(Target::Node),
            "browser" => Some(Target::Browser),
            "none" => Some(Target::None),
            _ => None,
        }
    }

    /// The target's display name, as accepted by `--target`.
    pub fn name(self) -> &'static str {
        match self {
            Target::Node => "node",
            Target::Browser => "browser",
            Target::None => "none",
        }
    }
}
