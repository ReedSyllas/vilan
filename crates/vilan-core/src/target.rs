//! The compilation target — which host the emitted JavaScript runs on. It
//! determines which platform `std` modules are reachable (a browser build can't
//! load `std::http`; a Node build can't load the DOM layer) and how host
//! bindings are emitted (Node's `process.exit` vs none in the browser).

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

/// Which platform layer a `std` module belongs to. `Core` is universal (loadable
/// for any target); `Node` and `Browser` are platform layers gated by the build
/// target. The flat module loader classifies by name rather than directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Core,
    Node,
    Browser,
}

impl Platform {
    /// The platform layer of the `std` module named `name`.
    pub fn of_std_module(name: &str) -> Platform {
        match name {
            // The Node layer: filesystem, process, and the HTTP server.
            "fs" | "http" | "process" => Platform::Node,
            // The browser (DOM) layer, and the reactive UI layer built on it.
            "dom" | "ui" => Platform::Browser,
            _ => Platform::Core,
        }
    }

    /// A human-readable name for diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            Platform::Core => "core",
            Platform::Node => "Node",
            Platform::Browser => "browser",
        }
    }

    /// Whether a module of this platform can be loaded for `target`. Core is
    /// universal; a platform layer only loads for its own target.
    pub fn is_available_for(self, target: Target) -> bool {
        match self {
            Platform::Core => true,
            Platform::Node => target == Target::Node,
            Platform::Browser => target == Target::Browser,
        }
    }
}
