//! The build configuration's two axes (see `proposal/platform-model.md`):
//!
//! - the **backend** — which emitter runs (the output language); JS only today.
//! - the **platform** — where the program runs: a host runtime + version, or
//!   `none` (type-check only). A library's *layers* declare which platforms they
//!   serve (`PlatformPattern`), and a build resolves each module to the
//!   most-specific matching layer.
//!
//! The supported set is small but extensible: platforms `node:24` / `deno:2` /
//! `bun:1` / `browser`, backend `js`. Adding a runtime, a version, or a backend is
//! a change here.

/// The supported Node major version (the current LTS) — the only `node` version
/// that builds for now.
pub const NODE_LTS: u32 = 24;

/// The supported Deno major version (the current major) — the only `deno` version
/// that builds for now.
pub const DENO_CURRENT: u32 = 2;

/// The supported Bun major version (the current major) — the only `bun` version
/// that builds for now.
pub const BUN_CURRENT: u32 = 1;

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
    /// Deno, by major version (only [`DENO_CURRENT`] is supported today). A
    /// process-having runtime (`@process`) like Node; its `node:`-compat bindings
    /// make the shared `process` layer serve it without a per-runtime split.
    Deno { version: u32 },
    /// Bun, by major version (only [`BUN_CURRENT`] is supported today). Another
    /// `@process` runtime with `node:` compatibility — same shared `process` layer.
    Bun { version: u32 },
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
    /// Parses a `--platform` value: `node` / `node:24` / `deno` / `deno:2` / `bun` /
    /// `bun:1` / `browser` / `none`. A process runtime with no version defaults to its
    /// current supported major; an unsupported version errors.
    pub fn parse(name: &str) -> Result<Self, String> {
        let (runtime, version) = match name.split_once(':') {
            Some((runtime, version)) => (runtime, Some(version)),
            None => (name, None),
        };
        match runtime {
            "node" => Ok(Platform::Node {
                version: supported_version(version, NODE_LTS, "node")?,
            }),
            "deno" => Ok(Platform::Deno {
                version: supported_version(version, DENO_CURRENT, "deno")?,
            }),
            "bun" => Ok(Platform::Bun {
                version: supported_version(version, BUN_CURRENT, "bun")?,
            }),
            "browser" | "none" if version.is_some() => {
                Err(format!("the `{runtime}` platform takes no version"))
            }
            "browser" => Ok(Platform::Browser),
            "none" => Ok(Platform::None),
            _ => Err(format!(
                "unknown platform `{name}` (expected `node`, `deno`, `bun`, `browser`, or `none`)"
            )),
        }
    }

    /// The platform's display name (`node:24` / `deno:2` / `bun:1` / `browser` /
    /// `none`).
    pub fn name(self) -> String {
        match self {
            Platform::Node { version } => format!("node:{version}"),
            Platform::Deno { version } => format!("deno:{version}"),
            Platform::Bun { version } => format!("bun:{version}"),
            Platform::Browser => "browser".to_string(),
            Platform::None => "none".to_string(),
        }
    }

    /// Whether this is the host-less `none` platform (check-only).
    pub fn is_none(self) -> bool {
        matches!(self, Platform::None)
    }

    /// Whether the host has `process.exit` (so `main`'s result becomes an exit
    /// code) — the one host-profile bit codegen needs. True for the process
    /// runtimes (Node, Deno, and Bun, via their `node:` compat).
    pub fn has_process_exit(self) -> bool {
        matches!(
            self,
            Platform::Node { .. } | Platform::Deno { .. } | Platform::Bun { .. }
        )
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
            )
            | (
                Platform::Deno { version },
                PlatformPattern::Deno {
                    version: Some(wanted),
                },
            )
            | (
                Platform::Bun { version },
                PlatformPattern::Bun {
                    version: Some(wanted),
                },
            ) if version == wanted => Some(2),
            (Platform::Node { .. }, PlatformPattern::Node { version: None })
            | (Platform::Deno { .. }, PlatformPattern::Deno { version: None })
            | (Platform::Bun { .. }, PlatformPattern::Bun { version: None })
            | (Platform::Browser, PlatformPattern::Browser) => Some(1),
            _ => None,
        }
    }
}

/// Validates a process runtime's version token: defaults to `supported` (the only
/// version that builds today) when omitted, errors on a non-numeric or unsupported
/// value. Shared by every versioned runtime so adding one doesn't duplicate this.
fn supported_version(version: Option<&str>, supported: u32, runtime: &str) -> Result<u32, String> {
    let version = match version {
        None => supported,
        Some(text) => text
            .parse()
            .map_err(|_| format!("invalid {runtime} version `{text}`"))?,
    };
    if version != supported {
        return Err(format!(
            "unsupported {runtime} version `{version}` (supported: {supported})"
        ));
    }
    Ok(version)
}

/// What platforms a library layer serves — a pattern. A process runtime's version
/// of `None` means "any version of that runtime"; `Some(v)` is a version-specific
/// override.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformPattern {
    Node { version: Option<u32> },
    Deno { version: Option<u32> },
    Bun { version: Option<u32> },
    Browser,
}

impl PlatformPattern {
    /// Parses one `[library.layer.<l>].platform` token into the patterns it covers,
    /// expanding families (`@process` → the process-having runtimes). `None` for an
    /// unknown token (a typo'd platform name).
    pub fn parse(token: &str) -> Option<Vec<PlatformPattern>> {
        // Families: a named set of runtimes, so a layer (and a new runtime) is a
        // one-line edit here, not per-library churn. `@process` is the process-having
        // runtimes — node, deno, and bun today.
        if token == "@process" {
            return Some(vec![
                PlatformPattern::Node { version: None },
                PlatformPattern::Deno { version: None },
                PlatformPattern::Bun { version: None },
            ]);
        }
        let (runtime, version) = match token.split_once(':') {
            Some((runtime, "*")) => (runtime, None),
            Some((runtime, text)) => (runtime, Some(text.parse().ok()?)),
            None => (token, None),
        };
        match runtime {
            "node" => Some(vec![PlatformPattern::Node { version }]),
            "deno" => Some(vec![PlatformPattern::Deno { version }]),
            "bun" => Some(vec![PlatformPattern::Bun { version }]),
            "browser" if version.is_none() => Some(vec![PlatformPattern::Browser]),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_platforms() {
        assert_eq!(
            Platform::parse("node"),
            Ok(Platform::Node { version: NODE_LTS })
        );
        assert_eq!(
            Platform::parse("node:24"),
            Ok(Platform::Node { version: 24 })
        );
        assert_eq!(
            Platform::parse("deno"),
            Ok(Platform::Deno {
                version: DENO_CURRENT
            })
        );
        assert_eq!(Platform::parse("deno:2"), Ok(Platform::Deno { version: 2 }));
        assert_eq!(
            Platform::parse("bun"),
            Ok(Platform::Bun {
                version: BUN_CURRENT
            })
        );
        assert_eq!(Platform::parse("bun:1"), Ok(Platform::Bun { version: 1 }));
        assert_eq!(Platform::parse("browser"), Ok(Platform::Browser));
        assert_eq!(Platform::parse("none"), Ok(Platform::None));
    }

    #[test]
    fn parse_rejects_unsupported_and_unknown() {
        assert!(
            Platform::parse("deno:9")
                .unwrap_err()
                .contains("unsupported deno version")
        );
        assert!(
            Platform::parse("node:18")
                .unwrap_err()
                .contains("unsupported node version")
        );
        // `nodejs` is a typo for `node` — never a valid runtime, so a stable "unknown".
        assert!(
            Platform::parse("nodejs")
                .unwrap_err()
                .contains("unknown platform")
        );
        assert!(
            Platform::parse("browser:1")
                .unwrap_err()
                .contains("takes no version")
        );
    }

    #[test]
    fn names_round_trip() {
        for platform in [
            Platform::Node { version: NODE_LTS },
            Platform::Deno {
                version: DENO_CURRENT,
            },
            Platform::Bun {
                version: BUN_CURRENT,
            },
            Platform::Browser,
            Platform::None,
        ] {
            assert_eq!(Platform::parse(&platform.name()), Ok(platform));
        }
    }

    #[test]
    fn process_family_expands_to_all_runtimes() {
        let patterns = PlatformPattern::parse("@process").unwrap();
        assert_eq!(
            patterns,
            vec![
                PlatformPattern::Node { version: None },
                PlatformPattern::Deno { version: None },
                PlatformPattern::Bun { version: None },
            ]
        );
        // A `process` layer (declared `@process`) matches every process runtime, but
        // not the browser — the whole point of the family.
        for runtime in [
            Platform::Node { version: NODE_LTS },
            Platform::Deno {
                version: DENO_CURRENT,
            },
            Platform::Bun {
                version: BUN_CURRENT,
            },
        ] {
            assert!(
                patterns.iter().any(|p| runtime.matches(*p).is_some()),
                "{} should match @process",
                runtime.name()
            );
        }
        assert!(
            patterns
                .iter()
                .all(|p| Platform::Browser.matches(*p).is_none())
        );
    }

    #[test]
    fn matching_is_runtime_specific_and_version_ranked() {
        let deno = Platform::Deno { version: 2 };
        // Exact version outranks any-version; a different runtime never matches.
        assert_eq!(
            deno.matches(PlatformPattern::Deno { version: Some(2) }),
            Some(2)
        );
        assert_eq!(
            deno.matches(PlatformPattern::Deno { version: None }),
            Some(1)
        );
        assert_eq!(deno.matches(PlatformPattern::Node { version: None }), None);
        assert_eq!(
            Platform::Node { version: NODE_LTS }.matches(PlatformPattern::Deno { version: None }),
            None
        );
    }

    #[test]
    fn process_runtimes_have_process_exit() {
        assert!(Platform::Node { version: NODE_LTS }.has_process_exit());
        assert!(
            Platform::Deno {
                version: DENO_CURRENT
            }
            .has_process_exit()
        );
        assert!(
            Platform::Bun {
                version: BUN_CURRENT
            }
            .has_process_exit()
        );
        assert!(!Platform::Browser.has_process_exit());
        assert!(!Platform::None.has_process_exit());
    }
}
