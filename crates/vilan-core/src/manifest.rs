//! The typed `vilan.toml` manifest: a declarative description of a *package* (a
//! buildable / importable unit) or a *project* (a workspace grouping packages).
//! Both the `vilan` CLI and the language server parse a manifest through here, so
//! the schema — and its validation — has a single source of truth.
//!
//! P1 makes resolution fully declarative (no inference): a package names its
//! source `root` (default `src`) and `entry` (default `main.vl`, resolved against
//! the root) and its default `target`. The workspace (`[project]`) and dependency
//! schema parse here too, but resolving them across packages is later work — see
//! `proposal/project-model-p1.md`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::analyzer::{PackageSpec, Workspace};
use crate::options::{BuildOptions, Preset};
use crate::target::Target;

/// A parsed `vilan.toml`. Exactly one of `[package]` / `[project]` is present for
/// a current-shape manifest; the legacy `[server]` + `[client]` pair (P2 replaces
/// it with per-package targets) is still accepted.
#[derive(Debug, Default, Deserialize)]
pub struct Manifest {
    pub package: Option<Package>,
    pub project: Option<Project>,
    pub build: Option<Build>,
    /// Legacy full-stack server entry (`[server]`), kept working through P1.
    pub server: Option<EntrySection>,
    /// Legacy full-stack client entry (`[client]`), kept working through P1.
    pub client: Option<EntrySection>,
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
    /// The default build target (`node` / `browser` / `none`). Default `node`.
    pub target: Option<String>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
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

    /// The declared target, if any (validated by [`Manifest::validate`]).
    pub fn resolved_target(&self) -> Option<Target> {
        self.target.as_deref().and_then(Target::parse)
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
        const KNOWN: &[&str] = &["package", "project", "build", "server", "client"];
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
            if self.package.is_some() || self.project.is_some() {
                errors.push(
                    "the legacy `[server]`/`[client]` form can't be combined with \
                     `[package]` or `[project]`"
                        .to_string(),
                );
            }
        } else {
            match (&self.package, &self.project) {
                (Some(_), Some(_)) => errors.push(
                    "set either `[package]` or `[project]`, not both — a package and \
                     a workspace root are different manifests"
                        .to_string(),
                ),
                (None, None) => errors
                    .push("`vilan.toml` must declare a `[package]` or a `[project]`".to_string()),
                _ => {}
            }
        }

        if let Some(package) = &self.package {
            self.validate_package(package, &mut errors);
        }
        for dependencies in [
            self.package.as_ref().map(|p| &p.dependencies),
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
            if Target::parse(target).is_none() {
                errors.push(format!(
                    "unknown `[package] target` `{target}` (expected `node`, `browser`, or `none`)"
                ));
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

/// Resolves the dependency [`Workspace`] for the package rooted at `package_dir`,
/// built for `target` (P2): every reachable local `path` dependency, transitively,
/// with cycle detection and the target-compatibility rule — a dependency must
/// target `none` or the build target. A directory whose manifest declares no
/// `[package]` (and a bare file, which has no manifest) yields an empty workspace.
/// Shared by the CLI and the language server so both resolve imports identically.
pub fn resolve_workspace(package_dir: &Path, target: Target) -> Result<Workspace, String> {
    let manifest = load_manifest(package_dir)?;
    let Some(package) = manifest.package else {
        return Ok(Workspace::default());
    };
    let mut packages = Vec::new();
    let mut index_by_path = HashMap::new();
    let mut visiting = HashSet::new();
    let entry_dependencies = resolve_dependency_edges(
        &package.dependencies,
        package_dir,
        target,
        &mut packages,
        &mut index_by_path,
        &mut visiting,
    )?;
    Ok(Workspace {
        packages,
        entry_dependencies,
    })
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
    target: Target,
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
        let package = manifest.package.ok_or_else(|| {
            format!(
                "dependency `{import_name}` at `{}` is not a `[package]`",
                dependency_dir.display()
            )
        })?;
        let dependency_target = package.resolved_target().unwrap_or(Target::Node);
        if dependency_target != Target::None && dependency_target != target {
            return Err(format!(
                "dependency `{import_name}` targets `{}`, which a `{}` build can't import \
                 (a dependency must target `none` or `{}`)",
                dependency_target.name(),
                target.name(),
                target.name()
            ));
        }
        // Resolve the dependency's own dependencies first, so they take lower
        // indices (a valid load order), then record the dependency itself.
        let dependency_edges = resolve_dependency_edges(
            &package.dependencies,
            &dependency_dir,
            target,
            packages,
            index_by_path,
            visiting,
        )?;
        visiting.remove(&canonical);
        let index = packages.len();
        packages.push(PackageSpec {
            root: dependency_dir.join(package.root()),
            target: dependency_target,
            dependencies: dependency_edges,
        });
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
        assert_eq!(package.resolved_target(), Some(Target::Browser));
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
        let manifest = parse("[package]\nname = \"x\"\ntarget = \"deno\"\n");
        assert!(manifest.validate().iter().any(|e| e.contains("target")));
    }

    #[test]
    fn target_none_is_valid() {
        let manifest = parse("[package]\nname = \"common\"\ntarget = \"none\"\n");
        assert_eq!(
            manifest.package.as_ref().unwrap().resolved_target(),
            Some(Target::None)
        );
        assert!(manifest.validate().is_empty());
    }

    #[test]
    fn package_and_project_are_mutually_exclusive() {
        let manifest = parse("[package]\nname = \"x\"\n[project]\npackages = []\n");
        assert!(manifest.validate().iter().any(|e| e.contains("not both")));
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
