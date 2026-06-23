//! The build configuration's two axes (see `proposal/platform-model.md`):
//!
//! - the **backend** — which emitter runs (the output language); JS only today.
//! - the **platform** — where the program runs: a host runtime + version, or
//!   `none` (type-check only). A library's *layers* declare which platforms they
//!   serve (`PlatformPattern`), and a build resolves each module to the
//!   most-specific matching layer.
//!
//! The supported set is small but extensible: platform `node:24`/`browser`,
//! backend `js`. Adding a runtime, a node version, or a backend is a change here.

/// The supported Node major version (the current LTS) — the only `node` version
/// that builds for now.
pub const NODE_LTS: u32 = 24;

/// The emitter backend — the output language. JavaScript today; WASM later.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Js,
}

impl Backend {
    /// Parses a `--backend` value (`js`), or `None` if unrecognized.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "js" => Some(Backend::Js),
            _ => None,
        }
    }

    /// The backend's name, as accepted by `--backend`.
    pub fn name(self) -> &'static str {
        match self {
            Backend::Js => "js",
        }
    }
}

/// A concrete build platform: a host runtime (with version), or `None` — no host,
/// for type-checking only (a pure library can't be built, only checked). This is
/// the build's identity that selects which library layers are reachable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    /// Node.js, by major version (only [`NODE_LTS`] is supported today).
    Node { version: u32 },
    /// The browser.
    Browser,
    /// No host — `vilan check` only; emitting requires a concrete host.
    None,
}

impl Default for Platform {
    fn default() -> Self {
        Platform::Node { version: NODE_LTS }
    }
}

impl Platform {
    /// Parses a `--platform` value: `node` / `node:24` / `browser` / `none`. A
    /// `node` with no version defaults to the LTS; an unsupported version errors.
    pub fn parse(name: &str) -> Result<Self, String> {
        let (runtime, version) = match name.split_once(':') {
            Some((runtime, version)) => (runtime, Some(version)),
            None => (name, None),
        };
        match runtime {
            "node" => {
                let version = match version {
                    None => NODE_LTS,
                    Some(text) => text
                        .parse()
                        .map_err(|_| format!("invalid node version `{text}`"))?,
                };
                if version != NODE_LTS {
                    return Err(format!(
                        "unsupported node version `{version}` (supported: {NODE_LTS})"
                    ));
                }
                Ok(Platform::Node { version })
            }
            "browser" | "none" if version.is_some() => {
                Err(format!("the `{runtime}` platform takes no version"))
            }
            "browser" => Ok(Platform::Browser),
            "none" => Ok(Platform::None),
            _ => Err(format!(
                "unknown platform `{name}` (expected `node`, `browser`, or `none`)"
            )),
        }
    }

    /// The platform's display name (`node:24` / `browser` / `none`).
    pub fn name(self) -> String {
        match self {
            Platform::Node { version } => format!("node:{version}"),
            Platform::Browser => "browser".to_string(),
            Platform::None => "none".to_string(),
        }
    }

    /// Whether this is the host-less `none` platform (check-only).
    pub fn is_none(self) -> bool {
        matches!(self, Platform::None)
    }

    /// Whether the host has `process.exit` (so `main`'s result becomes an exit
    /// code) — the one host-profile bit codegen needs.
    pub fn has_process_exit(self) -> bool {
        matches!(self, Platform::Node { .. })
    }

    /// How specifically this platform matches `pattern` (higher = more specific),
    /// or `None` if it doesn't match. An exact-version pattern outranks any-version.
    pub fn matches(self, pattern: PlatformPattern) -> Option<u8> {
        match (self, pattern) {
            (
                Platform::Node { version },
                PlatformPattern::Node {
                    version: Some(wanted),
                },
            ) if version == wanted => Some(2),
            (Platform::Node { .. }, PlatformPattern::Node { version: None }) => Some(1),
            (Platform::Browser, PlatformPattern::Browser) => Some(1),
            _ => None,
        }
    }
}

/// What platforms a library layer serves — a pattern. A `node` version of `None`
/// means "any node version"; `Some(v)` is a version-specific override.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformPattern {
    Node { version: Option<u32> },
    Browser,
}

impl PlatformPattern {
    /// Parses one `[library.layer.<l>].platform` token into the patterns it covers,
    /// expanding families (`@process` → the process-having runtimes). `None` for an
    /// unknown token (a typo'd platform name).
    pub fn parse(token: &str) -> Option<Vec<PlatformPattern>> {
        // Families: a named set of runtimes, so a layer (and a new runtime) is a
        // one-line edit here, not per-library churn. `@process` is node (plus deno
        // and bun once they're added).
        if token == "@process" {
            return Some(vec![PlatformPattern::Node { version: None }]);
        }
        let (runtime, version) = match token.split_once(':') {
            Some((runtime, "*")) => (runtime, None),
            Some((runtime, text)) => (runtime, Some(text.parse().ok()?)),
            None => (token, None),
        };
        match runtime {
            "node" => Some(vec![PlatformPattern::Node { version }]),
            "browser" if version.is_none() => Some(vec![PlatformPattern::Browser]),
            _ => None,
        }
    }
}
