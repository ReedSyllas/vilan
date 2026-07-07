//! The typed `vilan.toml` manifest: a declarative description of a *package* (an
//! app), a *library* (an importable, target-layered unit), or a *project* (a
//! workspace grouping members). Both the `vilan` CLI and the language server parse
//! a manifest through here, so the schema — and its validation — has a single
//! source of truth.
//!
//! P1 makes resolution fully declarative (no inference): a package names its
//! source `root` (default `src`) and `entry` (default `main.vl`, resolved against
//! the root) and its default `target`. The workspace (`[project]`) and dependency
//! schema parse here too, but resolving them across packages is later work — see
//! `proposal/project-model-p1.md`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::analyzer::{Layer, PackageSpec, Workspace};
use crate::options::{BuildOptions, Preset};
use crate::target::{Platform, PlatformPattern};

/// A parsed `vilan.toml`. Exactly one of `[package]` (an app) / `[library]` (an
/// importable library) / `[project]` (a workspace) is present for a current-shape
/// manifest; the legacy `[server]` + `[client]` pair (P2 replaces it with
/// per-package targets) is still accepted.
#[derive(Debug, Default, Deserialize)]
pub struct Manifest {
    pub package: Option<Package>,
    pub library: Option<Library>,
    pub project: Option<Project>,
    pub build: Option<Build>,
    /// `[macro]` — expansion budgets (macro-engine.md §5): `fuel` (interpreter
    /// steps per macro run, default 1_000_000) and `depth` (expansion fixpoint
    /// rounds, default 16).
    #[serde(rename = "macro", default)]
    pub macro_: Option<MacroSection>,
    /// Legacy full-stack server entry (`[server]`), kept working through P1.
    pub server: Option<EntrySection>,
    /// Legacy full-stack client entry (`[client]`), kept working through P1.
    pub client: Option<EntrySection>,
}

/// The `[macro]` section: per-package expansion budgets.
#[derive(Debug, Default, Deserialize)]
pub struct MacroSection {
    pub fuel: Option<u64>,
    pub depth: Option<u32>,
}

/// A package: a buildable, importable unit.
#[derive(Debug, Deserialize)]
pub struct Package {
    /// How other packages import this one (P2). Required; a valid identifier.
    pub name: Option<String>,
    pub description: Option<String>,
    /// The package's source root, relative to the manifest. Default `src`.
    pub root: Option<PathBuf>,
    /// The `build`/`run` entry, resolved against `root`. Default `main.vl`.
    pub entry: Option<PathBuf>,
    /// The default build platform (`node` / `deno` / `bun` / `browser` / `none`).
    /// Default `node`.
    pub target: Option<String>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
}

/// A library: an importable unit with a public surface (`lib.vl`) and no app
/// baggage — no `entry`, no single host `target`. It serves every platform by
/// **layering** its source: a base `root` (shared) plus `[library.layer.<name>]`
/// overlays that each declare the platforms they serve (a module there shadows the
/// base for those platforms). See `proposal/platform-model.md`.
#[derive(Debug, Deserialize)]
pub struct Library {
    /// How dependents import this library. Required; a valid identifier.
    pub name: Option<String>,
    pub description: Option<String>,
    /// The base (shared) source root, relative to the manifest. Default `src`.
    pub root: Option<PathBuf>,
    /// Overlay layers, keyed by layer name (`process`, `browser`, …).
    #[serde(default)]
    pub layer: BTreeMap<String, LayerDecl>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
}

/// One `[library.layer.<name>]`: a source root plus the platform patterns it serves.
#[derive(Debug, Deserialize)]
pub struct LayerDecl {
    /// The layer's source root, relative to the manifest. Defaults to `src/<name>`.
    pub root: Option<PathBuf>,
    /// The platforms this layer serves: `node` / `node:24` / `node:*` / `deno` /
    /// `bun` / `browser`, or a family (`@process`). At least one.
    #[serde(default)]
    pub platform: Vec<String>,
}

impl Library {
    /// The base source root (default `src`).
    pub fn base_root(&self) -> &Path {
        self.root.as_deref().unwrap_or(Path::new("src"))
    }
}

/// A workspace root: a set of member packages plus dependencies they inherit.
#[derive(Debug, Default, Deserialize)]
pub struct Project {
    #[serde(default)]
    pub packages: Vec<PathBuf>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
}

/// A legacy `[server]` / `[client]` section — only its `entry` is read.
#[derive(Debug, Deserialize)]
pub struct EntrySection {
    pub entry: Option<PathBuf>,
}

/// A dependency: either a bare version string (`dep = "1.2"`, a registry
/// dependency) or the table form (`{ version, registry, path }`). A `path` makes
/// it a local *path dependency*; otherwise it is a *registry dependency*.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Dependency {
    Version(String),
    Detailed {
        version: Option<String>,
        registry: Option<String>,
        path: Option<PathBuf>,
    },
}

impl Dependency {
    /// The local path, if this is a path dependency.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Dependency::Detailed {
                path: Some(path), ..
            } => Some(path),
            _ => None,
        }
    }

    /// A registry dependency is one without a local `path` — P1 can't resolve it.
    pub fn is_registry(&self) -> bool {
        self.path().is_none()
    }
}

/// The `[build]` code-generation knobs, deserialized before resolving against a
/// preset (see [`Manifest::build_options`]).
#[derive(Debug, Default, Deserialize)]
pub struct Build {
    pub preset: Option<String>,
    pub indent: Option<bool>,
    pub spaces: Option<bool>,
    #[serde(rename = "readable-names")]
    pub readable_names: Option<bool>,
    #[serde(rename = "debug-names")]
    pub debug_names: Option<bool>,
}

impl Package {
    /// The source root (default `src`).
    pub fn root(&self) -> &Path {
        self.root.as_deref().unwrap_or(Path::new("src"))
    }

    /// The entry file name, relative to the root (default `main.vl`).
    pub fn entry(&self) -> &Path {
        self.entry.as_deref().unwrap_or(Path::new("main.vl"))
    }

    /// The declared platform, if any (validated by [`Manifest::validate`]).
    pub fn resolved_target(&self) -> Option<Platform> {
        self.target.as_deref().and_then(|t| Platform::parse(t).ok())
    }
}

impl Manifest {
    /// Parses `vilan.toml` text into the typed schema. Returns the manifest plus
    /// non-fatal warnings (e.g. unknown top-level keys, which a forward-compatible
    /// reader ignores rather than rejects). Structural / type errors are `Err`.
    pub fn parse(text: &str) -> Result<(Manifest, Vec<String>), String> {
        let manifest: Manifest = toml::from_str(text).map_err(|error| error.to_string())?;
        // Unknown top-level keys are ignored (forward-compat), but worth flagging
        // so a typo doesn't silently do nothing. A second, untyped parse keeps the
        // typed deserialize free of a catch-all field.
        let table: toml::Table = toml::from_str(text).map_err(|error| error.to_string())?;
        const KNOWN: &[&str] = &["package", "library", "project", "build", "server", "client"];
        let warnings = table
            .keys()
            .filter(|key| !KNOWN.contains(&key.as_str()))
            .map(|key| format!("unknown `vilan.toml` key `{key}` (ignored)"))
            .collect();
        Ok((manifest, warnings))
    }

    /// Whether this is a legacy full-stack manifest (`[server]` + `[client]`).
    pub fn is_legacy_fullstack(&self) -> bool {
        self.server.is_some() && self.client.is_some()
    }

    /// Validates the schema, returning a (possibly empty) list of error messages.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let has_legacy = self.server.is_some() || self.client.is_some();

        if has_legacy {
            // The legacy shape needs both halves; a lone `[server]`/`[client]` is
            // an incomplete pair (use a `[package]` with a `target` instead).
            if self.server.is_none() || self.client.is_none() {
                errors.push(
                    "a `[server]` needs a matching `[client]` (and vice versa); \
                     for a single-target app use `[package]` with a `target`"
                        .to_string(),
                );
            }
            if self.package.is_some() || self.library.is_some() || self.project.is_some() {
                errors.push(
                    "the legacy `[server]`/`[client]` form can't be combined with \
                     `[package]`, `[library]`, or `[project]`"
                        .to_string(),
                );
            }
        } else {
            // Exactly one of `[package]` (app) / `[library]` / `[project]` (workspace).
            let kinds = self.package.is_some() as u8
                + self.library.is_some() as u8
                + self.project.is_some() as u8;
            if kinds > 1 {
                errors.push(
                    "set exactly one of `[package]`, `[library]`, or `[project]` — an app, a \
                     library, and a workspace root are different manifests"
                        .to_string(),
                );
            } else if kinds == 0 {
                errors.push(
                    "`vilan.toml` must declare a `[package]`, `[library]`, or `[project]`"
                        .to_string(),
                );
            }
        }

        if let Some(package) = &self.package {
            self.validate_package(package, &mut errors);
        }
        if let Some(library) = &self.library {
            self.validate_library(library, &mut errors);
        }
        for dependencies in [
            self.package.as_ref().map(|p| &p.dependencies),
            self.library.as_ref().map(|l| &l.dependencies),
            self.project.as_ref().map(|p| &p.dependencies),
        ]
        .into_iter()
        .flatten()
        {
            validate_dependencies(dependencies, &mut errors);
        }
        if let Some(build) = &self.build {
            if let Some(preset) = &build.preset {
                if Preset::parse(preset).is_none() {
                    errors.push(format!(
                        "unknown build preset `{preset}` (expected `debug` or `release`)"
                    ));
                }
            }
        }
        errors
    }

    fn validate_package(&self, package: &Package, errors: &mut Vec<String>) {
        match &package.name {
            None => errors.push("`[package]` is missing a `name`".to_string()),
            Some(name) if !is_identifier(name) => errors.push(format!(
                "`[package] name` must be a valid identifier (got `{name}`)"
            )),
            Some(_) => {}
        }
        if let Some(target) = &package.target {
            if let Err(error) = Platform::parse(target) {
                errors.push(format!("invalid `[package] target`: {error}"));
            }
        }
    }

    fn validate_library(&self, library: &Library, errors: &mut Vec<String>) {
        match &library.name {
            None => errors.push("`[library]` is missing a `name`".to_string()),
            Some(name) if !is_identifier(name) => errors.push(format!(
                "`[library] name` must be a valid identifier (got `{name}`)"
            )),
            Some(_) => {}
        }
        for (name, layer) in &library.layer {
            if layer.platform.is_empty() {
                errors.push(format!(
                    "`[library.layer.{name}]` must declare the platforms it serves \
                     (e.g. `platform = [\"@process\"]`)"
                ));
            }
            for token in &layer.platform {
                if PlatformPattern::parse(token).is_none() {
                    errors.push(format!(
                        "`[library.layer.{name}]` has an unknown platform `{token}` \
                         (expected `node`/`node:24`/`deno`/`bun`/`browser`, or `@process`)"
                    ));
                }
            }
        }
    }

    /// Resolves the `[build]` options: a `preset` (default `debug`) initializes
    /// every option, then individual keys override it.
    pub fn build_options(&self) -> Result<BuildOptions, String> {
        let Some(build) = &self.build else {
            return Ok(BuildOptions::default());
        };
        let mut options = match &build.preset {
            Some(name) => BuildOptions::from_preset(Preset::parse(name).ok_or_else(|| {
                format!("unknown build preset `{name}` (expected `debug` or `release`)")
            })?),
            None => BuildOptions::default(),
        };
        options.indent = build.indent.unwrap_or(options.indent);
        options.spaces = build.spaces.unwrap_or(options.spaces);
        options.readable_names = build.readable_names.unwrap_or(options.readable_names);
        options.debug_names = build.debug_names.unwrap_or(options.debug_names);
        Ok(options)
    }
}

/// Rejects registry dependencies (P1 resolves neither; path dependencies parse
/// but load later — see the roadmap). Reported as errors so a declared dependency
/// is never silently ignored.
fn validate_dependencies(dependencies: &BTreeMap<String, Dependency>, errors: &mut Vec<String>) {
    for (name, dependency) in dependencies {
        if dependency.is_registry() {
            errors.push(format!(
                "registry dependency `{name}` is not yet supported \
                 (only local `path` dependencies are recognized)"
            ));
        }
    }
}

/// Resolves the dependency [`Workspace`] for the package rooted at `package_dir`
/// (P2): every reachable local `path` dependency, transitively, with cycle
/// detection. Each `PackageSpec` records its declared `target`, but the graph
/// itself is target-independent — the target-compatibility *diagnostic* is the
/// analyzer's, reported at the import (P3). A directory whose manifest declares no
/// `[package]` (and a bare file, which has no manifest) yields an empty workspace.
/// Shared by the CLI and the language server so both resolve imports identically.
pub fn resolve_workspace(package_dir: &Path) -> Result<Workspace, String> {
    let manifest = load_manifest(package_dir)?;
    let defaults = crate::macros::MacroLimits::default();
    let macro_limits = manifest
        .macro_
        .as_ref()
        .map(|section| crate::macros::MacroLimits {
            fuel: section.fuel.unwrap_or(defaults.fuel),
            depth: section.depth.unwrap_or(defaults.depth),
        })
        .unwrap_or(defaults);
    let Some(package) = manifest.package else {
        return Ok(Workspace {
            macro_limits,
            ..Workspace::default()
        });
    };
    let mut packages = Vec::new();
    let mut index_by_path = HashMap::new();
    let mut visiting = HashSet::new();
    let entry_dependencies = resolve_dependency_edges(
        &package.dependencies,
        package_dir,
        &mut packages,
        &mut index_by_path,
        &mut visiting,
    )?;
    Ok(Workspace {
        packages,
        entry_dependencies,
        macro_limits,
    })
}

/// Resolves a `[library]`'s layered [`PackageSpec`] from its package directory `dir`
/// (with its `vilan.toml`). A directory with no manifest is a base-only library (its
/// own `dir` is the base layer). Dependency edges are left empty — this resolves the
/// library's *own* layer structure (for `std`, and for the platform contract check),
/// not a full dependency build.
pub fn resolve_library(dir: &Path) -> PackageSpec {
    if let Ok(contents) = std::fs::read_to_string(dir.join("vilan.toml")) {
        if let Ok((manifest, _)) = Manifest::parse(&contents) {
            if let Some(library) = manifest.library {
                return library_spec(dir, &library, Vec::new());
            }
        }
    }
    PackageSpec {
        base_root: dir.to_path_buf(),
        layers: Vec::new(),
        dependencies: Vec::new(),
    }
}

/// Resolves the `std` library's spec — `std` is just a library, so this is
/// [`resolve_library`] at the std package directory. Point `VILAN_STD` at that
/// directory (not the bare `src`) to get the platform layers.
pub fn resolve_std(std_dir: &Path) -> PackageSpec {
    resolve_library(std_dir)
}

/// Builds a [`PackageSpec`] for the `[library]` rooted at `dir`: its base root
/// (default `src`) plus each declared layer (root default `src/<name>`, with the
/// platform patterns it serves), and the already-resolved dependency edges.
fn library_spec(dir: &Path, library: &Library, dependencies: Vec<(String, usize)>) -> PackageSpec {
    let layers = library
        .layer
        .iter()
        .map(|(name, decl)| {
            let root = decl
                .root
                .clone()
                .unwrap_or_else(|| PathBuf::from("src").join(name));
            let patterns = decl
                .platform
                .iter()
                .filter_map(|token| PlatformPattern::parse(token))
                .flatten()
                .collect();
            Layer {
                name: name.clone(),
                patterns,
                root: dir.join(root),
            }
        })
        .collect();
    PackageSpec {
        base_root: dir.join(library.base_root()),
        layers,
        dependencies,
    }
}

/// Reads, parses, and validates the `vilan.toml` in `directory` (for dependency
/// resolution — warnings are the front-end's concern and are dropped here).
fn load_manifest(directory: &Path) -> Result<Manifest, String> {
    let manifest_path = directory.join("vilan.toml");
    let contents = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    let (manifest, _warnings) = Manifest::parse(&contents)
        .map_err(|error| format!("invalid {}: {error}", manifest_path.display()))?;
    let errors = manifest.validate();
    if !errors.is_empty() {
        return Err(format!(
            "invalid {}:\n  - {}",
            manifest_path.display(),
            errors.join("\n  - ")
        ));
    }
    Ok(manifest)
}

/// Resolves one package's `path` dependency edges to `(import name, index)` pairs,
/// loading each referenced package (transitively) into `packages`. `index_by_path`
/// dedups a shared dependency; `visiting` is the in-progress stack for cycle
/// detection. Paths are relative to `base_dir` (the depending package's directory).
fn resolve_dependency_edges(
    dependencies: &BTreeMap<String, Dependency>,
    base_dir: &Path,
    packages: &mut Vec<PackageSpec>,
    index_by_path: &mut HashMap<PathBuf, usize>,
    visiting: &mut HashSet<PathBuf>,
) -> Result<Vec<(String, usize)>, String> {
    let mut edges = Vec::new();
    for (import_name, dependency) in dependencies {
        // `validate` rejects registry dependencies, so only `path` deps reach here.
        let Some(relative) = dependency.path() else {
            continue;
        };
        let dependency_dir = base_dir.join(relative);
        let canonical =
            std::fs::canonicalize(&dependency_dir).unwrap_or_else(|_| dependency_dir.clone());
        if let Some(&index) = index_by_path.get(&canonical) {
            edges.push((import_name.clone(), index));
            continue;
        }
        if !visiting.insert(canonical.clone()) {
            return Err(format!(
                "dependency cycle through `{}`",
                dependency_dir.display()
            ));
        }
        let manifest = load_manifest(&dependency_dir)
            .map_err(|error| format!("dependency `{import_name}`: {error}"))?;
        // A dependency must be a `[library]` — you depend on libraries, not apps
        // (L1, Q2). A `[package]` (app) dependency is an error with a migration hint.
        let library = match (&manifest.library, &manifest.package) {
            (Some(_), _) => manifest.library.unwrap(),
            (None, Some(_)) => {
                return Err(format!(
                    "dependency `{import_name}` at `{}` is a `[package]` (an app); only \
                     `[library]` packages can be depended on — change its `[package]` to a \
                     `[library]`",
                    dependency_dir.display()
                ));
            }
            (None, None) => {
                return Err(format!(
                    "dependency `{import_name}` at `{}` is not a `[library]`",
                    dependency_dir.display()
                ));
            }
        };
        // Resolve the library's own dependencies first, so they take lower indices
        // (a valid load order), then record the library itself. Its layered roots (a
        // base plus each declared per-target overlay) come from `library_spec`; a
        // target-specific module being unavailable for the build target is the
        // analyzer's per-module diagnostic at the import (L1), not a resolution error.
        let dependency_edges = resolve_dependency_edges(
            &library.dependencies,
            &dependency_dir,
            packages,
            index_by_path,
            visiting,
        )?;
        visiting.remove(&canonical);
        let index = packages.len();
        packages.push(library_spec(&dependency_dir, &library, dependency_edges));
        index_by_path.insert(canonical, index);
        edges.push((import_name.clone(), index));
    }
    Ok(edges)
}

/// Whether `name` is a valid Vilan identifier: a leading letter or `_`, then
/// letters, digits, or `_`.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Manifest {
        Manifest::parse(text).expect("parses").0
    }

    #[test]
    fn package_defaults() {
        let manifest = parse("[package]\nname = \"math\"\n");
        let package = manifest.package.as_ref().unwrap();
        assert_eq!(package.root(), Path::new("src"));
        assert_eq!(package.entry(), Path::new("main.vl"));
        assert_eq!(package.resolved_target(), None);
        assert!(manifest.validate().is_empty());
    }

    #[test]
    fn package_explicit_fields() {
        let manifest = parse(
            "[package]\nname = \"web\"\nroot = \"source\"\nentry = \"app.vl\"\ntarget = \"browser\"\n",
        );
        let package = manifest.package.as_ref().unwrap();
        assert_eq!(package.root(), Path::new("source"));
        assert_eq!(package.entry(), Path::new("app.vl"));
        assert_eq!(package.resolved_target(), Some(Platform::Browser));
        assert!(manifest.validate().is_empty());
    }

    #[test]
    fn missing_name_is_an_error() {
        let manifest = parse("[package]\ntarget = \"node\"\n");
        assert!(manifest.validate().iter().any(|e| e.contains("name")));
    }

    #[test]
    fn non_identifier_name_is_an_error() {
        let manifest = parse("[package]\nname = \"my-pkg\"\n");
        assert!(manifest.validate().iter().any(|e| e.contains("identifier")));
    }

    #[test]
    fn unknown_target_is_an_error() {
        let manifest = parse("[package]\nname = \"x\"\ntarget = \"nodejs\"\n");
        assert!(manifest.validate().iter().any(|e| e.contains("target")));
    }

    #[test]
    fn target_none_is_valid() {
        let manifest = parse("[package]\nname = \"common\"\ntarget = \"none\"\n");
        assert_eq!(
            manifest.package.as_ref().unwrap().resolved_target(),
            Some(Platform::None)
        );
        assert!(manifest.validate().is_empty());
    }

    #[test]
    fn deno_target_is_valid() {
        let manifest = parse("[package]\nname = \"svc\"\ntarget = \"deno\"\n");
        assert_eq!(
            manifest.package.as_ref().unwrap().resolved_target(),
            Platform::parse("deno").ok()
        );
        assert!(manifest.validate().is_empty());
    }

    #[test]
    fn library_layer_serving_deno_is_valid() {
        let manifest =
            parse("[library]\nname = \"x\"\n[library.layer.deno]\nplatform = [\"deno\"]\n");
        assert!(manifest.validate().is_empty());
    }

    /// Whether any layer in `spec` serves a platform matching `pattern`.
    fn serves(spec: &PackageSpec, pattern: PlatformPattern) -> bool {
        spec.layers
            .iter()
            .any(|layer| layer.patterns.iter().any(|p| *p == pattern))
    }

    #[test]
    fn resolve_std_reads_manifest_layers() {
        // The real `std` library declares `process`/`browser` layers in its manifest.
        let std = resolve_std(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std"));
        assert!(std.base_root.ends_with("std/src"));
        assert!(serves(&std, PlatformPattern::Node { version: None }));
        assert!(serves(&std, PlatformPattern::Browser));
    }

    #[test]
    fn resolve_std_without_manifest_is_base_only() {
        // Pointing at a bare source root (no `vilan.toml`) yields a base-only library:
        // its `src` is the base layer and there are no platform layers. A `VILAN_STD`
        // at the src root still resolves the core modules, just not the overlays.
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std/src");
        let std = resolve_std(&src);
        assert_eq!(std.base_root, src);
        assert!(std.layers.is_empty());
    }

    #[test]
    fn package_and_project_are_mutually_exclusive() {
        let manifest = parse("[package]\nname = \"x\"\n[project]\npackages = []\n");
        assert!(
            manifest
                .validate()
                .iter()
                .any(|e| e.contains("exactly one"))
        );
    }

    #[test]
    fn package_and_library_are_mutually_exclusive() {
        let manifest = parse("[package]\nname = \"x\"\n[library]\nname = \"y\"\n");
        assert!(
            manifest
                .validate()
                .iter()
                .any(|e| e.contains("exactly one"))
        );
    }

    #[test]
    fn library_with_layer_is_valid() {
        let manifest = parse(
            "[library]\nname = \"geometry\"\n[library.layer.process]\nplatform = [\"@process\"]\n",
        );
        let library = manifest.library.as_ref().unwrap();
        assert_eq!(library.base_root(), Path::new("src"));
        assert!(library.layer.contains_key("process"));
        assert!(manifest.validate().is_empty());
    }

    #[test]
    fn library_missing_name_is_an_error() {
        let manifest = parse("[library]\n");
        assert!(manifest.validate().iter().any(|e| e.contains("name")));
    }

    #[test]
    fn library_layer_without_platform_is_an_error() {
        // A layer must declare the platforms it serves — the layer *name* is free
        // (it doesn't imply a platform), so an empty `platform` is ambiguous.
        let manifest = parse("[library]\nname = \"x\"\n[library.layer.weird]\nroot = \"w\"\n");
        assert!(manifest.validate().iter().any(|e| e.contains("weird")));
    }

    #[test]
    fn unknown_library_layer_platform_is_an_error() {
        let manifest =
            parse("[library]\nname = \"x\"\n[library.layer.l]\nplatform = [\"nodejs\"]\n");
        assert!(manifest.validate().iter().any(|e| e.contains("nodejs")));
    }

    #[test]
    fn neither_section_is_an_error() {
        let manifest = parse("[build]\npreset = \"release\"\n");
        assert!(
            manifest
                .validate()
                .iter()
                .any(|e| e.contains("must declare"))
        );
    }

    #[test]
    fn registry_dependency_is_rejected() {
        let manifest =
            parse("[package]\nname = \"x\"\n[package.dependencies]\ngeometry = \"1.2\"\n");
        assert!(
            manifest
                .validate()
                .iter()
                .any(|e| e.contains("registry dependency"))
        );
    }

    #[test]
    fn path_dependency_is_accepted() {
        let manifest = parse(
            "[package]\nname = \"x\"\n[package.dependencies]\nshapes = { path = \"../shapes\" }\n",
        );
        assert!(manifest.validate().is_empty());
        assert_eq!(
            manifest.package.as_ref().unwrap().dependencies["shapes"].path(),
            Some(Path::new("../shapes"))
        );
    }

    #[test]
    fn legacy_fullstack_parses() {
        let manifest = parse("[server]\nentry = \"server.vl\"\n[client]\nentry = \"client.vl\"\n");
        assert!(manifest.is_legacy_fullstack());
        assert!(manifest.validate().is_empty());
    }

    #[test]
    fn lone_client_is_an_error() {
        let manifest = parse("[client]\nentry = \"app.vl\"\n");
        assert!(manifest.validate().iter().any(|e| e.contains("matching")));
    }

    #[test]
    fn build_options_from_preset_and_overrides() {
        let manifest = parse(
            "[package]\nname = \"x\"\n[build]\npreset = \"release\"\nreadable-names = true\n",
        );
        let options = manifest.build_options().unwrap();
        assert!(!options.indent); // release
        assert!(options.readable_names); // overridden on
    }

    #[test]
    fn unknown_top_level_key_warns() {
        let (_, warnings) = Manifest::parse("[package]\nname = \"x\"\n[wat]\nk = 1\n").unwrap();
        assert!(warnings.iter().any(|w| w.contains("wat")));
    }
}
