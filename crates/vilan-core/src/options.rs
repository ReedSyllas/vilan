//! Code-generation options — what the emitted JavaScript looks like. Resolved
//! from a `preset` (a named starting point that sets every option) plus
//! per-feature overrides, configured under `[build]` in a project's `vilan.toml`.
//!
//! The two modes are independent: a readable **debug** build (indented, with
//! source-name annotations, fewer optimizations) and an optimized **release**
//! build (minified, obfuscated). New optimization knobs are added here as fields
//! with a default in each preset.

/// A named starting point that initializes every option; individual options in
/// `[build]` then override it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// Readable output for debugging: indented, source-name annotations.
    Debug,
    /// Optimized output for deployment: minified, obfuscated.
    Release,
}

impl Preset {
    /// Parses a `preset = "..."` value, or `None` if unrecognized.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "debug" => Some(Preset::Debug),
            "release" => Some(Preset::Release),
            _ => None,
        }
    }

    /// The preset's name, as written in `vilan.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Debug => "debug",
            Preset::Release => "release",
        }
    }
}

/// The resolved set of code-generation options for a build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildOptions {
    /// Indent and space the output for readability. Off: a minified one-liner.
    pub indent: bool,
    /// Annotate generated identifiers with their source name (`a/*count*/`) so the
    /// output is debuggable. Off: obfuscated short names only.
    pub debug_names: bool,
}

impl BuildOptions {
    /// The options a preset starts from, before any per-feature override.
    pub fn from_preset(preset: Preset) -> Self {
        match preset {
            Preset::Debug => Self {
                indent: true,
                debug_names: true,
            },
            Preset::Release => Self {
                indent: false,
                debug_names: false,
            },
        }
    }
}

impl Default for BuildOptions {
    /// Debug — the readable default (matches a plain `vilan build`).
    fn default() -> Self {
        Self::from_preset(Preset::Debug)
    }
}
