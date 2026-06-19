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
    /// Lay the output out across lines with indentation (vs one flat line). Off:
    /// no line breaks, no leading indent.
    pub indent: bool,
    /// Pad tokens with spaces — around operators, after commas, inside array
    /// brackets (`a + b`, `[ 1, 2 ]`). Off: tight (`a+b`, `[1,2]`).
    pub spaces: bool,
    /// Name generated identifiers after their source (a function `greet` becomes
    /// `greet`, disambiguated on collision) — the most debuggable output. Off: see
    /// `debug_names`. Takes precedence over `debug_names` when on.
    pub readable_names: bool,
    /// When `readable_names` is off, still annotate the obfuscated short names with
    /// their source (`a/*count*/`). Off: obfuscated short names only.
    pub debug_names: bool,
}

impl BuildOptions {
    /// The options a preset starts from, before any per-feature override.
    pub fn from_preset(preset: Preset) -> Self {
        match preset {
            Preset::Debug => Self {
                indent: true,
                spaces: true,
                readable_names: true,
                debug_names: false,
            },
            Preset::Release => Self {
                indent: false,
                spaces: false,
                readable_names: false,
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
