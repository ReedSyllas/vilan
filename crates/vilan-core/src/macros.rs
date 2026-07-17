//! The macro engine's expansion layer (proposal/macro-engine.md, Phase 1):
//! `macro fun` definitions compile in per-file HERMETIC worlds and run in the
//! expansion interpreter; `[name(args)]` attributes (and `[derive(Name)]` for
//! registered names) splice each macro's returned `Source` text into the
//! program before analysis.
//!
//! The macro world (§3): a file's `macro fun`s are compiled from a BLANKED
//! copy of that file — every byte outside the macro definitions becomes a
//! space (newlines kept), and the `macro` keyword itself is blanked, leaving
//! plain `fun`s at their original offsets. Spans in world diagnostics therefore
//! point at the true positions in the user's file. The world's package
//! universe is `macro_std` alone (a workspace with that single dependency);
//! hermeticity of the bodies is checked syntactically here (imports must root
//! at `macro_std`) and physically by construction (nothing else of the user's
//! program exists in the world).
//!
//! Caching (§6): worlds are cached by blanked-file content hash; expansions by
//! (world, macro, item source, argument sources) — sound because the
//! interpreter is deterministic by construction. Both caches hold leaked,
//! process-global data, mirroring `load_package_module`'s parse cache.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::analyzer::SourceId;
use crate::error::Error;
use crate::id::Id;
use crate::interpreter::{self, Limits};
use crate::node::{Func, ImportBranch, Node, NodeList, Pattern};
use crate::options::BuildOptions;
use crate::span::{Span, Spanned};
use crate::transformer::{JsProgram, js, transform_functions};
use crate::{PackageSpec, Platform, Workspace, analyze_source};

/// The derive names the RUST generators still serve when no macro is in
/// scope (fixture stds without the std macros; the macro world's own nested
/// compile). Frozen byte-identical copies of the migrated macros.
const RUST_DERIVES: &[&str] = &["PartialEq", "Default", "Debug", "Json", "Wire", "Hashable"];

/// The per-package expansion budgets (`vilan.toml [macro]`, macro-engine.md
/// §5/§12): `fuel` bounds one macro run's interpreter steps; `depth` bounds
/// the expansion fixpoint (output carrying further invocations). Defaults are
/// the settled review values.
#[derive(Debug, Clone, Copy)]
pub struct MacroLimits {
    pub fuel: u64,
    pub depth: u32,
}

impl Default for MacroLimits {
    fn default() -> Self {
        Self {
            fuel: 1_000_000,
            depth: 16,
        }
    }
}

/// A compiled per-file macro world: the transformed program the interpreter
/// executes, plus each macro's emitted entry name and parameter count.
pub(crate) struct World {
    key: u64,
    program: JsProgram<'static>,
    /// macro name → its emitted function name.
    entries: HashMap<String, String>,
}

/// A module's identity for macro scoping: which package, which module. Macro
/// NAMES distribute through the module system (macro-engine.md §3) — a macro
/// is in scope in the file that defines it, in files that import it by leaf
/// (`import pkg::x::my_macro`), and — for macros defined in `std` modules —
/// everywhere (the macro PRELUDE: the built-in derives are ambient vocabulary,
/// like the primitive types).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum ModuleKey {
    /// The entry file.
    Entry,
    /// A std module, by name (its macros are the prelude).
    Std(String),
    /// An entry-package sibling module, by name.
    Pkg(String),
    /// A dependency's module: (package index, module name).
    Dep(usize, String),
    /// A dependency's `lib.vl` (its root-level surface), by package index.
    DepLib(usize),
}

#[derive(Default)]
pub(crate) struct MacroRegistry {
    by_module: HashMap<ModuleKey, HashMap<String, MacroDef>>,
    /// `macro { .. }` blocks per file, keyed by the block NODE's address —
    /// anonymous, so they dispatch by position, not name (macro-engine.md
    /// Phase 4). Only the defining file's own blocks are ever in scope.
    blocks_by_module: HashMap<ModuleKey, HashMap<usize, MacroDef>>,
}

impl MacroRegistry {
    fn module(&self, key: &ModuleKey) -> Option<&HashMap<String, MacroDef>> {
        self.by_module.get(key)
    }
}

/// The macros in scope for ONE file: same-file definitions shadow imported
/// ones, which shadow the std prelude — the ordinary name-resolution order.
/// `blocks` are the file's OWN `macro { .. }` blocks, by node address.
pub(crate) struct MacroScope<'r> {
    names: HashMap<String, &'r MacroDef>,
    blocks: Option<&'r HashMap<usize, MacroDef>>,
}

impl<'r> MacroScope<'r> {
    fn get(&self, name: &str) -> Option<&'r MacroDef> {
        self.names.get(name).copied()
    }

    fn block(&self, address: usize) -> Option<&'r MacroDef> {
        self.blocks.and_then(|blocks| blocks.get(&address))
    }
}

/// Builds a file's macro scope: the std prelude, then the file's imports
/// (any depth — H2 imports are statements; macro visibility is file-flat),
/// then the file's own macros. `package` locates the file for `pkg::` and
/// dependency-root resolution.
pub(crate) fn scope_for<'r>(
    registry: &'r MacroRegistry,
    workspace: &Workspace,
    package: &FilePackage,
    key: &ModuleKey,
    nodes: &NodeList,
) -> MacroScope<'r> {
    let mut names: HashMap<String, &MacroDef> = HashMap::new();
    // 1. The std prelude.
    for (module_key, macros) in &registry.by_module {
        if matches!(module_key, ModuleKey::Std(_)) {
            for (name, def) in macros {
                names.insert(name.clone(), def);
            }
        }
    }
    // 2. The file's imports, resolved to registered macros by leaf name.
    let mut imports: Vec<(Vec<&str>, &str)> = Vec::new();
    fn collect_imports<'a>(node: &'a Spanned<Node<'a>>, out: &mut Vec<(Vec<&'a str>, &'a str)>) {
        if let Node::Import(branch) | Node::Use(branch) = &node.0 {
            let mut entries = Vec::new();
            crate::analyzer::flatten_namespace_branch(branch, Vec::new(), &mut entries);
            for (path, leaf, _leaf_span) in entries {
                out.push((path.iter().map(|(name, _)| *name).collect(), leaf));
            }
        }
        node.0
            .for_each_child(&mut |child| collect_imports(child, out));
    }
    for node in nodes {
        collect_imports(node, &mut imports);
    }
    for (path, leaf) in imports {
        let Some(root) = path.first().copied() else {
            continue;
        };
        let module = path.get(1).copied();
        let target: Option<ModuleKey> = match root {
            "std" => module.map(|module| ModuleKey::Std(module.to_string())),
            "pkg" => match package {
                FilePackage::Entry => module.map(|module| ModuleKey::Pkg(module.to_string())),
                FilePackage::Std => module.map(|module| ModuleKey::Std(module.to_string())),
                FilePackage::Dep(index) => {
                    module.map(|module| ModuleKey::Dep(*index, module.to_string()))
                }
            },
            dependency => {
                let edges = match package {
                    FilePackage::Entry => &workspace.entry_dependencies,
                    FilePackage::Std => return_none_edges(),
                    FilePackage::Dep(index) => &workspace.packages[*index].dependencies,
                };
                edges
                    .iter()
                    .find(|(name, _)| name == dependency)
                    .map(|(_, index)| match module {
                        Some(module) => ModuleKey::Dep(*index, module.to_string()),
                        None => ModuleKey::DepLib(*index),
                    })
            }
        };
        if let Some(target) = target {
            if let Some(def) = registry.module(&target).and_then(|macros| macros.get(leaf)) {
                names.insert(leaf.to_string(), def);
            }
        }
    }
    // 3. The file's own macros (highest precedence).
    if let Some(own) = registry.module(key) {
        for (name, def) in own {
            names.insert(name.clone(), def);
        }
    }
    MacroScope {
        names,
        blocks: registry.blocks_by_module.get(key),
    }
}

/// Which package a file belongs to, for import-root resolution.
pub(crate) enum FilePackage {
    Entry,
    Std,
    Dep(usize),
}

/// std files have no dependency edges.
fn return_none_edges() -> &'static Vec<(String, usize)> {
    static EMPTY: Vec<(String, usize)> = Vec::new();
    &EMPTY
}

/// What a macro's signature says it consumes — which dispatch forms fit.
/// Attributes need `(Item)` / `(Item, Arguments)`; invocations need
/// `(Arguments)` / `()`.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum MacroShape {
    Item,
    ItemArguments,
    Arguments,
    Unit,
}

pub(crate) struct MacroDef {
    /// The macro's declared name (the world's entry lookup key).
    name: String,
    /// `None` = a HELPER (§3: helpers are macro funs) — compiled into the
    /// world and callable from other macros, but not dispatchable from
    /// program code (its signature is not a macro shape, or it doesn't
    /// return `Source`). Computed syntactically at registration.
    shape: Option<MacroShape>,
    /// The file's blanked world source (shared by the file's macros) and its
    /// location — the world compiles LAZILY on first dispatch (registration
    /// stays syntactic, so a program pays only for the macros it uses), cached
    /// process-globally by blanked content.
    blanked: Arc<String>,
    file: PathBuf,
    /// The defining file's source in THIS analysis — world-compile errors
    /// attribute here (their spans point into the defining file).
    source: SourceId,
    world: std::cell::RefCell<Option<Arc<World>>>,
}

impl MacroDef {
    /// The compiled world and this macro's emitted entry name; compiles on
    /// first use. `Err` = the world's diagnostics (spans in `self.file`).
    #[allow(clippy::wrong_self_convention)]
    fn world(&self, std: &PackageSpec) -> Result<(Arc<World>, String), Vec<Error>> {
        let existing = self.world.borrow().clone();
        let world = match existing {
            Some(world) => world,
            None => {
                let Some(macro_std) = resolve_macro_std(std) else {
                    return Err(vec![Error {
                        note: None,
                        span: (0..0).into(),
                        msg: "the `macro_std` package was not found beside `std`".to_string(),
                    }]);
                };
                let world = compile_world((*self.blanked).clone(), &self.file, std, &macro_std)?;
                *self.world.borrow_mut() = Some(world.clone());
                world
            }
        };
        let Some(entry) = world.entries.get(&self.name).cloned() else {
            return Err(vec![Error {
                note: None,
                span: (0..0).into(),
                msg: format!(
                    "the macro `{}` did not compile to a callable function",
                    self.name
                ),
            }]);
        };
        Ok((world, entry))
    }
}

/// The `macro_std` package, resolved from its toolchain location beside `std`
/// (`<roots>/std/src` → `<roots>/macro_std`). `None` when absent — an error is
/// reported only when a program actually defines a macro.
pub(crate) fn resolve_macro_std(std: &PackageSpec) -> Option<PackageSpec> {
    let dir = std.base_root.parent()?.parent()?.join("macro_std");
    dir.join("vilan.toml")
        .is_file()
        .then(|| crate::manifest::resolve_std(&dir))
}

/// The top-level `macro fun`s of a file, with each definition's full span.
fn macro_funs<'a, 'src>(nodes: &'a NodeList<'src>) -> Vec<(&'a Func<'src>, Span)> {
    nodes
        .iter()
        .filter_map(|(node, span)| match node {
            Node::MacroFun(function) => Some((function, *span)),
            Node::Export(inner) => match &inner.0 {
                Node::MacroFun(function) => Some((function, inner.1)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// A file's `macro { .. }` blocks, at ANY depth (expression position nests),
/// with each block's node address and full span — plus the spans of ILLEGAL
/// ones (inside a `macro fun` body or another block: those bodies are already
/// macro-world code, where there is nothing to splice into).
fn macro_blocks<'a, 'src>(nodes: &'a NodeList<'src>) -> (Vec<(usize, Span)>, Vec<Span>) {
    fn walk<'a, 'src>(
        node: &'a Spanned<Node<'src>>,
        inside_macro: bool,
        blocks: &mut Vec<(usize, Span)>,
        illegal: &mut Vec<Span>,
    ) {
        let inside_next = match &node.0 {
            Node::MacroFun(_) => true,
            Node::MacroBlock(_) => {
                if inside_macro {
                    illegal.push(node.1);
                } else {
                    blocks.push((node as *const Spanned<Node> as usize, node.1));
                }
                true
            }
            _ => inside_macro,
        };
        node.0
            .for_each_child(&mut |child| walk(child, inside_next, blocks, illegal));
    }
    let mut blocks = Vec::new();
    let mut illegal = Vec::new();
    for node in nodes {
        walk(node, false, &mut blocks, &mut illegal);
    }
    blocks.sort_by_key(|(_, span)| span.into_range().start);
    (blocks, illegal)
}

/// The world-side entry name of the file's `n`-th block (text order) — the
/// same numbering `analyze_source`'s world hook stamps onto the synthetic
/// wrapper funs.
pub(crate) fn block_entry_name(ordinal: usize) -> String {
    format!("__macro_block_{ordinal}")
}

/// Registers a file's `macro fun`s: checks their bodies' hermeticity, compiles
/// the file's macro world (cached by blanked content), and binds each macro
/// name. Diagnostics carry spans into THIS file (the caller attributes them).
pub(crate) fn register_file(
    registry: &mut MacroRegistry,
    key: ModuleKey,
    nodes: &NodeList,
    text: &str,
    file: &Path,
    source: SourceId,
    std: &PackageSpec,
    diagnostics: &mut Vec<Error>,
) {
    let definitions = macro_funs(nodes);
    let (blocks, illegal_blocks) = macro_blocks(nodes);
    for span in &illegal_blocks {
        diagnostics.push(Error {
            note: None,
            span: *span,
            msg: "a `macro { .. }` block cannot appear inside macro code — the enclosing \
                  body already runs at expansion time"
                .to_string(),
        });
    }
    if definitions.is_empty() && blocks.is_empty() {
        return;
    }
    let first_span = definitions
        .first()
        .map(|(_, span)| *span)
        .or_else(|| blocks.first().map(|(_, span)| *span))
        .unwrap_or_else(|| (0..0).into());
    let Some(macro_std) = resolve_macro_std(std) else {
        diagnostics.push(Error {
            note: None,
            span: first_span,
            msg: "the `macro_std` package was not found beside `std` — macros need the \
                  toolchain's `macro_std`"
                .to_string(),
        });
        return;
    };

    // Hermeticity (§4): a macro body imports only from `macro_std`. Checked on
    // the ORIGINAL parse so the error points at the offending import. Block
    // bodies are macro bodies too.
    let mut hermetic = illegal_blocks.is_empty();
    for (function, _) in &definitions {
        if let Some(body) = &function.body {
            for statement in body.0.0.iter().chain(std::iter::once(&*body.0.1)) {
                check_hermetic_imports(statement, diagnostics, &mut hermetic);
            }
        }
    }
    for node in nodes {
        check_hermetic_block_imports(node, diagnostics, &mut hermetic);
    }
    if !hermetic {
        return;
    }

    let _ = macro_std; // presence checked above; the world resolves it lazily
    let block_spans: Vec<Span> = blocks.iter().map(|(_, span)| *span).collect();
    let blanked = Arc::new(blank_to_world(text, &definitions, &block_spans));
    let module = registry.by_module.entry(key.clone()).or_default();
    for (function, _) in &definitions {
        let name = function.name.0.to_string();
        if module.contains_key(&name) {
            diagnostics.push(Error {
                note: None,
                span: function.name.1,
                msg: format!("a macro named `{name}` is already defined in this module"),
            });
            continue;
        }
        module.insert(
            name.clone(),
            MacroDef {
                name,
                shape: macro_shape(function),
                blanked: blanked.clone(),
                file: file.to_path_buf(),
                source,
                world: std::cell::RefCell::new(None),
            },
        );
    }
    if !blocks.is_empty() {
        let by_address = registry.blocks_by_module.entry(key).or_default();
        for (ordinal, (address, _)) in blocks.iter().enumerate() {
            by_address.insert(
                *address,
                MacroDef {
                    name: block_entry_name(ordinal),
                    shape: Some(MacroShape::Unit),
                    blanked: blanked.clone(),
                    file: file.to_path_buf(),
                    source,
                    world: std::cell::RefCell::new(None),
                },
            );
        }
    }
}

/// The hermetic-import check over `macro { .. }` block bodies found at any
/// depth (the check for `macro fun` bodies runs separately, from the
/// definitions list).
fn check_hermetic_block_imports(
    node: &Spanned<Node>,
    diagnostics: &mut Vec<Error>,
    hermetic: &mut bool,
) {
    match &node.0 {
        // A macro fun's body is checked by the caller; blocks inside it are
        // already illegal.
        Node::MacroFun(_) => {}
        Node::MacroBlock(body) => {
            for statement in body.0.0.iter().chain(std::iter::once(&*body.0.1)) {
                check_hermetic_imports(statement, diagnostics, hermetic);
            }
        }
        _ => {
            node.0.for_each_child(&mut |child| {
                check_hermetic_block_imports(child, diagnostics, hermetic)
            });
        }
    }
}

fn check_hermetic_imports(node: &Spanned<Node>, diagnostics: &mut Vec<Error>, hermetic: &mut bool) {
    if let Node::Import(branch) | Node::Use(branch) = &node.0 {
        let root = match branch {
            ImportBranch::Path(root, _, _) => Some(*root),
            ImportBranch::Set(_) => None,
        };
        if root != Some("macro_std") {
            diagnostics.push(Error {
                note: None,
                span: node.1,
                msg: "a macro body may import only from `macro_std` — the macro world is \
                      hermetic (macro-engine.md §4)"
                    .to_string(),
            });
            *hermetic = false;
        }
    }
    node.0
        .for_each_child(&mut |child| check_hermetic_imports(child, diagnostics, hermetic));
}

/// The dispatch shape a macro's written signature declares, by its parameter
/// TYPE names (`Item` / `Arguments`). `None` for anything else.
fn macro_shape(function: &Func) -> Option<MacroShape> {
    // A dispatchable macro returns `Source`; anything else is a helper.
    let returns_source = matches!(
        function.return_type.as_deref().map(|spanned| &spanned.0),
        Some(Node::Accessor("Source"))
    );
    if !returns_source {
        return None;
    }
    let type_name = |index: usize| -> Option<&str> {
        let (_, type_, _, _) = function.parameters.0.get(index)?;
        match type_.as_deref().map(|spanned| &spanned.0) {
            Some(Node::Accessor(name)) => Some(*name),
            _ => None,
        }
    };
    match function.parameters.0.len() {
        0 => Some(MacroShape::Unit),
        1 => match type_name(0)? {
            "Item" => Some(MacroShape::Item),
            "Arguments" => Some(MacroShape::Arguments),
            _ => None,
        },
        2 => match (type_name(0)?, type_name(1)?) {
            ("Item", "Arguments") => Some(MacroShape::ItemArguments),
            _ => None,
        },
        _ => None,
    }
}

/// The macro world's source: every byte outside the macro definitions becomes
/// a space (newlines kept, so spans and line numbers stay true), and each
/// definition's leading `macro` keyword is blanked, leaving a plain `fun`.
/// `macro { .. }` blocks stay VERBATIM (keyword included): the world's parse
/// sees them as `MacroBlock` statements at its top level, and the world hook
/// in `analyze_source` wraps each into a synthetic `fun __macro_block_<n>():
/// Source` — true spans, no offset arithmetic.
fn blank_to_world(text: &str, definitions: &[(&Func, Span)], blocks: &[Span]) -> String {
    let bytes = text.as_bytes();
    let mut blanked: Vec<u8> = bytes
        .iter()
        .map(|&b| if b == b'\n' { b'\n' } else { b' ' })
        .collect();
    for (_, span) in definitions {
        let range = span.into_range();
        let range = range.start.min(bytes.len())..range.end.min(bytes.len());
        blanked[range.clone()].copy_from_slice(&bytes[range.clone()]);
        // `macro` → 5 spaces (the definition span starts at the keyword).
        if bytes[range.clone()].starts_with(b"macro") {
            blanked[range.start..range.start + 5].fill(b' ');
        }
    }
    for span in blocks {
        let range = span.into_range();
        let range = range.start.min(bytes.len())..range.end.min(bytes.len());
        blanked[range.clone()].copy_from_slice(&bytes[range.clone()]);
    }
    String::from_utf8(blanked).expect("blanking preserves UTF-8 (multibyte bytes become spaces)")
}

fn content_key(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

thread_local! {
    /// Set while a macro WORLD is being analyzed. A world's own analysis must
    /// not register macros (std's prelude modules contain `macro fun`s —
    /// registering them would recursively compile their worlds, unboundedly);
    /// expansion still runs there, with an empty scope, so std's own derives
    /// generate through the byte-identical Rust fallback.
    static IN_MACRO_WORLD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn in_macro_world() -> bool {
    IN_MACRO_WORLD.with(|flag| flag.get())
}

/// The macro world's AMBIENT prelude vocabulary (macro-engine.md §3/§10): the
/// compiler-interaction surface — the `meta` reflection types plus
/// `source`/`fresh` — is in scope in every macro body without imports, the
/// way the derive macros are ambient in program code. Libraries (`option`,
/// `build`, …) stay explicit imports; the ambient set is exactly the surface
/// a macro exists to talk to.
const AMBIENT_META_TYPES: &[&str] = &[
    "Item",
    "StructItem",
    "EnumItem",
    "FunctionItem",
    "ServiceItem",
    "Field",
    "Variant",
    "TypeExpr",
    "Arguments",
    "Source",
];
const AMBIENT_FUNCTIONS: &[&str] = &["source", "fresh"];

/// The prelude's parsed import nodes for one world, with any name the file
/// DEFINES itself left out — a same-named `macro fun` shadows the prelude
/// (imports overwrite hoisted function bindings, so the exclusion is how the
/// prelude yields). Parsed fresh per world compile (worlds cache
/// process-globally); the built text leaks, bounded like the world itself.
pub(crate) fn world_prelude_nodes(
    defined: &std::collections::HashSet<&str>,
) -> Option<NodeList<'static>> {
    let survivors: Vec<&str> = AMBIENT_META_TYPES
        .iter()
        .copied()
        .filter(|name| !defined.contains(name))
        .collect();
    let mut text = String::new();
    if !survivors.is_empty() {
        text.push_str("import macro_std::meta::{ ");
        text.push_str(&survivors.join(", "));
        text.push_str(" };\n");
    }
    for function in AMBIENT_FUNCTIONS {
        if !defined.contains(function) {
            text.push_str("import macro_std::");
            text.push_str(function);
            text.push_str(";\n");
        }
    }
    if text.is_empty() {
        return Some(Vec::new());
    }
    let text: &'static str = Box::leak(text.into_boxed_str());
    let (root, _span) = crate::parse_clean(text)?;
    Some(root)
}

fn compile_world(
    blanked: String,
    file: &Path,
    std: &PackageSpec,
    macro_std: &PackageSpec,
) -> Result<Arc<World>, Vec<Error>> {
    static WORLDS: OnceLock<Mutex<HashMap<u64, Arc<World>>>> = OnceLock::new();
    let key = content_key(&blanked);
    let worlds = WORLDS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(world) = worlds.lock().unwrap().get(&key) {
        return Ok(world.clone());
    }

    let leaked: &'static str = Box::leak(blanked.into_boxed_str());
    let workspace = Workspace {
        packages: vec![macro_std.clone()],
        entry_dependencies: vec![("macro_std".to_string(), 0)],
        macro_limits: MacroLimits::default(),
    };
    let previously_in_world = IN_MACRO_WORLD.with(|flag| flag.replace(true));
    let (program, errors) = analyze_source(
        leaked,
        std,
        // A macro world is not a package: the package root is the FILE itself
        // (never matches a std layer root — that detection would flip the
        // world into compiling-std mode and skip the dependency registration
        // `macro_std` resolves through; stray `pkg::` imports root at a
        // non-directory, and the hermetic check bans them anyway). The ENTRY
        // path is synthetic: a std-hosted macro file is ALSO an always-loaded
        // std module, and a real path would alias that module to the blanked
        // entry (`is_entry_module`), walking the macros into a module scope
        // instead of the world's global one.
        file,
        &file.with_extension("vl.macro-world"),
        Some(Platform::default()),
        &workspace,
    );
    IN_MACRO_WORLD.with(|flag| flag.set(previously_in_world));
    if !errors.is_empty() {
        return Err(errors
            .into_iter()
            .map(|error| Error {
                note: None,
                span: error.span,
                msg: format!("in this macro: {}", error.msg),
            })
            .collect());
    }
    let Some(program) = program else {
        return Err(vec![Error {
            note: None,
            span: (0..0).into(),
            msg: "the macro world failed to compile".to_string(),
        }]);
    };
    // The world outlives this analysis (it's cached process-globally), so the
    // program is leaked — bounded: one leak per distinct macro-definition set,
    // the same shape as the parse cache's leaks.
    let program: &'static crate::Program<'static> = Box::leak(Box::new(program));

    // The world's roots are the blanked entry's top-level functions — exactly
    // the file's macro funs (and their helper macro funs).
    let mut roots = Vec::new();
    let mut root_names = Vec::new();
    if let Some(global) = program.scopes.get(&program.global_scope_id) {
        let mut named: Vec<(&str, Id)> = global
            .name_to_id_map
            .iter()
            .map(|(name, id)| (*name, *id))
            .filter(|(_, id)| program.functions.contains_key(id))
            .collect();
        named.sort_by_key(|(_, id)| id.0);
        for (name, id) in named {
            roots.push(id);
            root_names.push(name.to_string());
        }
    }
    let (js_program, emitted) = transform_functions(program, &BuildOptions::default(), &roots)
        .map_err(|error| vec![error])?;
    let entries = roots
        .iter()
        .zip(root_names)
        .map(|(id, name)| (name, emitted.get(id).cloned().unwrap_or_default()))
        .collect();
    let world = Arc::new(World {
        key,
        program: js_program,
        entries,
    });
    worlds.lock().unwrap().insert(key, world.clone());
    Ok(world)
}

// --- Expansion ---

/// One file's expansion results, ready for `analyze` to fold in.
#[derive(Default)]
pub(crate) struct ExpansionOutput {
    /// Generated ITEM lists, each with its ORIGIN — the span of the
    /// attribute/invocation in the user's file that produced it — walked
    /// after the originating file's items. The origin is what a diagnostic
    /// raised INSIDE the generated code re-anchors to (standard A2).
    pub(crate) items: Vec<(Span, &'static NodeList<'static>)>,
    /// Expression splices: invocation node address → the replacement
    /// expression the walk substitutes.
    pub(crate) expressions: Vec<(usize, &'static Spanned<Node<'static>>)>,
    /// Item-position invocation node addresses (they walk to nothing).
    pub(crate) item_sites: Vec<usize>,
    /// Expression sites whose expansion FAILED — the failure is already a
    /// diagnostic; the walk substitutes an error entity without piling on.
    pub(crate) failed_sites: Vec<usize>,
    /// Diagnostics whose spans point into a macro's DEFINING file (lazy world
    /// compiles fail at first use) — attributed per source by the caller.
    pub(crate) world_errors: Vec<(SourceId, Error)>,
}

struct Expander<'r, 'd> {
    scope: &'r MacroScope<'r>,
    std: &'r PackageSpec,
    limits: MacroLimits,
    /// Rust-generated fallback text (derive/service names with no macro in
    /// scope — fixture stds without the std macros). Flushed as ONE list,
    /// prelude-first, ahead of the macro-generated lists — the shape the
    /// pre-unification channel produced.
    rust_source: String,
    rust_traits: std::collections::HashSet<&'static str>,
    rust_any_service: bool,
    diagnostics: &'d mut Vec<Error>,
    /// The per-splice-site counter that stamps `__m<N>` gensym placeholders
    /// unique (§7): deterministic — sites are visited in file/node order.
    site_counter: &'d mut u32,
    output: ExpansionOutput,
}

/// Expands every macro use in `nodes` (a file's top level, or a generated list
/// when recursing): attributes and item-position invocations append generated
/// items; expression-position invocations (found at ANY depth, except inside
/// macro definitions) record their spliced replacement. Nested uses in
/// generated code are chased to the depth cap.
pub(crate) fn expand_source(
    scope: &MacroScope,
    std: &PackageSpec,
    limits: MacroLimits,
    nodes: &NodeList,
    text: &str,
    diagnostics: &mut Vec<Error>,
    site_counter: &mut u32,
    depth: u32,
) -> ExpansionOutput {
    let mut expander = Expander {
        scope,
        std,
        limits,
        rust_source: String::new(),
        rust_traits: std::collections::HashSet::new(),
        rust_any_service: false,
        diagnostics,
        site_counter,
        output: ExpansionOutput::default(),
    };
    expander.expand_list(nodes, text, depth);
    expander.flush_rust_fallback();
    expander.output
}

impl Expander<'_, '_> {
    fn expand_list(&mut self, nodes: &NodeList, text: &str, depth: u32) {
        for node in nodes {
            self.expand_item_position(node, nodes, text, depth);
        }
    }

    /// A node in ITEM position: attributes, item invocations, derives — and a
    /// sweep of everything else for expression-position invocations.
    fn expand_item_position(
        &mut self,
        node: &Spanned<Node>,
        siblings: &NodeList,
        text: &str,
        depth: u32,
    ) {
        match &node.0 {
            Node::Export(inner) => self.expand_item_position(inner, siblings, text, depth),
            // `mod` bodies are item position too (a service there gathers its
            // rpc surface from the mod's own items).
            Node::Module(_, body) => {
                for child in &body.0 {
                    self.expand_item_position(child, &body.0, text, depth);
                }
            }
            // A macro definition: its body is the macro world's, never
            // expanded (splice syntax is program-code-only).
            Node::MacroFun(_) => {}
            Node::MacroAttribute(name, name_span, argument_spans, item) => {
                let arguments: Vec<&str> = argument_spans
                    .iter()
                    .map(|span| slice(text, *span))
                    .collect();
                self.run_attribute(name, *name_span, item, &arguments, text, depth);
                // The annotated item may contain expression invocations.
                self.sweep_expressions(item, text, depth);
            }
            // `[derive(Name)]`: a macro named `Name` in scope dispatches like
            // an attribute with no arguments; the historical built-in names
            // fall back to the Rust generators when no macro is in scope
            // (fixture stds); unknown names keep today's behavior (skip — the
            // missing impl surfaces at the use site).
            Node::Derive(names, item) => {
                for (name, name_span) in names.iter() {
                    if self.scope.get(name).is_some() {
                        self.run_attribute(name, *name_span, item, &[], text, depth);
                    } else if let Some(known) =
                        RUST_DERIVES.iter().find(|known| **known == *name).copied()
                    {
                        self.rust_traits.insert(known);
                        self.rust_source
                            .push_str(&crate::analyzer::derive_impl_source(&[name], item));
                    }
                }
                self.sweep_expressions(item, text, depth);
            }
            // `[service(Client)]`: the std `service` macro (in the prelude) —
            // or the Rust generator when absent. The compiler gathers the
            // same-module [rpc] surface either way.
            Node::Service(client_name, item) => {
                match self.scope.get("service") {
                    Some(def) => {
                        self.run_service(def, *client_name, item, siblings, text, depth);
                    }
                    None => {
                        // The Rust generator exists for FIXTURE stds that have
                        // no rpc module at all. A real std reaching here means
                        // `std::rpc` wasn't loaded before this expansion — the
                        // B21 ordering class, whose symptom (a silently STALE
                        // twin of the macro's template) is far worse than a
                        // loud error. Every `[service]` site now seeds the rpc
                        // load (entry, load loop, dependency surfaces), so
                        // this firing again is a compiler bug to report.
                        if self.std.base_root.join("rpc.vl").is_file() {
                            self.diagnostics.push(Error {
                                note: None,
                                span: item.1,
                                msg: "`[service]` expanded before std::rpc's `service` macro was \
                                      loaded — a compiler load-ordering bug (B21's class); please \
                                      report how this module is reached"
                                    .to_string(),
                            });
                        } else {
                            self.rust_any_service = true;
                            self.rust_source
                                .push_str(&crate::analyzer::service_impl_source(
                                    *client_name,
                                    item,
                                    siblings,
                                ));
                        }
                    }
                }
                self.sweep_expressions(item, text, depth);
            }
            // An ITEM invocation: the output parses as items and appends.
            Node::MacroInvocation(name, name_span, argument_spans) => {
                let arguments: Vec<&str> = argument_spans
                    .iter()
                    .map(|span| slice(text, *span))
                    .collect();
                self.output
                    .item_sites
                    .push(node as *const Spanned<Node> as usize);
                self.run_invocation(name, *name_span, &arguments, depth, None);
            }
            // An ITEM-position `macro { .. }` block: the output parses as
            // items and appends (its body never expands here — world code).
            Node::MacroBlock(_) => {
                let site = node as *const Spanned<Node> as usize;
                self.output.item_sites.push(site);
                self.run_block(site, node.1, depth, None);
            }
            // Everything else: hunt expression-position invocations inside.
            _ => self.sweep_expressions(node, text, depth),
        }
    }

    /// Finds `macro name(..)` invocations and `macro { .. }` blocks in
    /// EXPRESSION position anywhere under `node` (macro definitions and block
    /// bodies excluded — those belong to the world).
    fn sweep_expressions(&mut self, node: &Spanned<Node>, text: &str, depth: u32) {
        match &node.0 {
            Node::MacroFun(_) => {}
            Node::MacroInvocation(name, name_span, argument_spans) => {
                let arguments: Vec<&str> = argument_spans
                    .iter()
                    .map(|span| slice(text, *span))
                    .collect();
                let site = node as *const Spanned<Node> as usize;
                self.run_invocation(name, *name_span, &arguments, depth, Some(site));
            }
            Node::MacroBlock(_) => {
                let site = node as *const Spanned<Node> as usize;
                self.run_block(site, node.1, depth, Some(site));
            }
            _ => {
                node.0
                    .for_each_child(&mut |child| self.sweep_expressions(child, text, depth));
            }
        }
    }

    /// `macro { .. }` dispatch: an anonymous zero-argument macro, resolved by
    /// the block node's ADDRESS in the defining file (blocks have no names).
    /// A miss means the file's registration failed (generated blocks are
    /// rejected at their generating site before this can run).
    fn run_block(
        &mut self,
        address: usize,
        site: Span,
        depth: u32,
        expression_site: Option<usize>,
    ) {
        let Some(def) = self.scope.block(address) else {
            self.diagnostics.push(Error {
                note: None,
                span: site,
                msg: "this `macro { .. }` block was not registered — see the file's \
                      earlier macro errors"
                    .to_string(),
            });
            if let Some(site_key) = expression_site {
                self.output.failed_sites.push(site_key);
            }
            return;
        };
        let name = def.name.clone();
        self.expand_call(
            def,
            &name,
            site,
            "",
            &[],
            Vec::new(),
            depth,
            expression_site,
        );
    }

    /// `[service(..)]` dispatch through the std `service` macro.
    fn run_service(
        &mut self,
        def: &MacroDef,
        client_name: Option<&str>,
        item: &Spanned<Node>,
        siblings: &NodeList,
        text: &str,
        depth: u32,
    ) {
        let Some((literal, input)) = construct_service(client_name, item, siblings, text) else {
            return; // a bodyless struct generates nothing, like the Rust path
        };
        // The input text (struct + gathered methods) is the cache key: leak it
        // to reuse expand_call's borrowed-slice shape.
        let input: &'static str = Box::leak(input.into_boxed_str());
        self.expand_call(
            def,
            "service",
            item.1,
            input,
            &[],
            vec![literal],
            depth,
            None,
        );
    }

    /// Flushes the Rust-generated fallback text (if any) as the FIRST items
    /// list, prefixed with the trait-import prelude the Rust generators
    /// assume — exactly the pre-unification channel's shape.
    fn flush_rust_fallback(&mut self) {
        if self.rust_source.trim().is_empty() {
            return;
        }
        let mut prelude = String::new();
        if self.rust_traits.contains("PartialEq") {
            prelude.push_str("import std::compare::PartialEq;\n");
        }
        if self.rust_traits.contains("Default") {
            prelude.push_str("import std::default::Default;\n");
        }
        if self.rust_traits.contains("Json") || self.rust_traits.contains("Wire") {
            // Mirrors the `Json`/`Wire` macro entry points: the validating
            // `from_json` yields a `Result` (I3), so the output needs `Result`
            // in scope; it reads JSON through methods (`try_parse_json`,
            // `has_field`), so no `parse_json_value`/`panic` import.
            prelude.push_str("import std::json::{ Json, FromJson, JsonValue };\n");
            prelude.push_str("import std::result::Result;\n");
        }
        if self.rust_traits.contains("Wire") {
            prelude.push_str(
                "import std::wire::{ Wire, Serialize, Deserialize, Serializer, Deserializer };\n",
            );
        }
        if self.rust_traits.contains("Debug") {
            prelude.push_str("import std::debug::Debug;\n");
        }
        if self.rust_traits.contains("Hashable") {
            prelude.push_str("import std::hash::{ Hashable, Hash, canonical_hash };\n");
        }
        if self.rust_any_service {
            prelude.push_str(
                "import std::rpc::{ Transport, Dispatcher, RpcError, RpcOutcome, RemoteSource, call, arg, reply, decode_failed, session_of, connect_socket, SocketTransport, bridge, ReactiveClient };\n",
            );
            prelude.push_str("import std::wire::{ Codec, Serializer };\n");
            prelude.push_str("import std::result::Result;\n");
            prelude.push_str("import std::option::Option;\n");
        }
        let combined = format!("{prelude}{}", self.rust_source);
        match parse_generated(&combined) {
            Ok((parsed, _)) => self.output.items.insert(0, ((0..0).into(), parsed)),
            Err(message) => self.diagnostics.push(Error {
                note: None,
                span: (0..0).into(),
                msg: format!("the built-in derive generators produced invalid vilan ({message})"),
            }),
        }
    }

    /// Attribute/derive dispatch: the macro must be Item-shaped.
    fn run_attribute(
        &mut self,
        name: &str,
        site: Span,
        item: &Spanned<Node>,
        arguments: &[&str],
        text: &str,
        depth: u32,
    ) {
        let Some(def) = self.scope.get(name) else {
            self.diagnostics.push(Error {
                note: None,
                span: site,
                msg: format!("no macro named `{name}` is in scope"),
            });
            return;
        };
        let call_arguments = match def.shape {
            None => {
                self.diagnostics.push(Error {
                    note: None,
                    span: site,
                    msg: format!(
                        "`{name}` is a macro HELPER (its signature is not a macro shape or it \
                         doesn't return `Source`) — only other macros can call it"
                    ),
                });
                return;
            }
            Some(MacroShape::Item) => vec![construct_item(item, text)],
            Some(MacroShape::ItemArguments) => {
                vec![construct_item(item, text), construct_arguments(arguments)]
            }
            Some(MacroShape::Arguments) | Some(MacroShape::Unit) => {
                self.diagnostics.push(Error {
                    note: None,
                    span: site,
                    msg: format!(
                        "macro `{name}` is invocation-shaped (it takes no `Item`) — call it \
                         as `macro {name}(..)`, not as an attribute"
                    ),
                });
                return;
            }
        };
        let item_text = slice(text, item.1);
        self.expand_call(
            def,
            name,
            site,
            item_text,
            arguments,
            call_arguments,
            depth,
            None,
        );
    }

    /// Invocation dispatch: the macro must be Arguments-shaped (or take
    /// nothing). `expression_site` is the invocation node's address when in
    /// expression position; `None` in item position.
    fn run_invocation(
        &mut self,
        name: &str,
        site: Span,
        arguments: &[&str],
        depth: u32,
        expression_site: Option<usize>,
    ) {
        let Some(def) = self.scope.get(name) else {
            self.diagnostics.push(Error {
                note: None,
                span: site,
                msg: format!("no macro named `{name}` is in scope"),
            });
            if let Some(site_key) = expression_site {
                self.output.failed_sites.push(site_key);
            }
            return;
        };
        let call_arguments = match def.shape {
            None => {
                self.diagnostics.push(Error {
                    note: None,
                    span: site,
                    msg: format!(
                        "`{name}` is a macro HELPER (its signature is not a macro shape or it \
                         doesn't return `Source`) — only other macros can call it"
                    ),
                });
                if let Some(site_key) = expression_site {
                    self.output.failed_sites.push(site_key);
                }
                return;
            }
            Some(MacroShape::Arguments) => vec![construct_arguments(arguments)],
            Some(MacroShape::Unit) => Vec::new(),
            Some(MacroShape::Item) | Some(MacroShape::ItemArguments) => {
                self.diagnostics.push(Error {
                    note: None,
                    span: site,
                    msg: format!(
                        "macro `{name}` is attribute-shaped (it takes an `Item`) — use it \
                         as `[{name}]` on an item, not as an invocation"
                    ),
                });
                if let Some(site_key) = expression_site {
                    self.output.failed_sites.push(site_key);
                }
                return;
            }
        };
        self.expand_call(
            def,
            name,
            site,
            "",
            arguments,
            call_arguments,
            depth,
            expression_site,
        );
    }

    /// Runs one macro call: the raw-text expansion cache, per-site gensym
    /// stamping, parsing (as items, or as one expression for an expression
    /// site), and the nested-expansion recursion.
    #[allow(clippy::too_many_arguments)]
    fn expand_call(
        &mut self,
        def: &MacroDef,
        name: &str,
        site: Span,
        item_text: &str,
        arguments: &[&str],
        call_arguments: Vec<js::Node<'static>>,
        depth: u32,
        expression_site: Option<usize>,
    ) {
        // Blocks are anonymous — their synthetic entry names would read as
        // noise in diagnostics.
        let label = if name.starts_with("__macro_block_") {
            "the `macro { .. }` block".to_string()
        } else {
            format!("macro `{name}`")
        };
        if depth >= self.limits.depth {
            let cap = self.limits.depth;
            self.diagnostics.push(Error {
                note: None,
                span: site,
                msg: format!(
                    "macro expansion did not settle after {cap} rounds — the chain ends \
                     at {label}"
                ),
            });
            if let Some(site_key) = expression_site {
                self.output.failed_sites.push(site_key);
            }
            return;
        }

        // The RAW output is cached by (world, macro, item source, argument
        // sources) — §6; sound because the interpreter is deterministic.
        // Gensym stamping is per SITE, so it applies after the cache.
        // Lazy world compile — errors carry the DEFINING file's spans.
        let (world, entry) = match def.world(self.std) {
            Ok(resolved) => resolved,
            Err(errors) => {
                self.output
                    .world_errors
                    .extend(errors.into_iter().map(|error| (def.source, error)));
                self.diagnostics.push(Error {
                    note: None,
                    span: site,
                    msg: format!("{label}'s definition did not compile"),
                });
                if let Some(site_key) = expression_site {
                    self.output.failed_sites.push(site_key);
                }
                return;
            }
        };
        let raw: &'static str = match cached_run(
            &world,
            &entry,
            name,
            item_text,
            arguments,
            &call_arguments,
            self.limits.fuel,
        ) {
            Ok(raw) => raw,
            Err(message) => {
                self.diagnostics.push(Error {
                    note: None,
                    span: site,
                    msg: format!("{label} failed at expansion time: {message}"),
                });
                if let Some(site_key) = expression_site {
                    self.output.failed_sites.push(site_key);
                }
                return;
            }
        };

        // Stamp `__m<N>` placeholders unique per splice site (§7). Outputs
        // without placeholders (the common case) parse through the
        // content-addressed cache; stamped text is site-unique, so it parses
        // fresh.
        *self.site_counter += 1;
        let stamped = stamp_gensyms(raw, *self.site_counter);

        if let Some(site_key) = expression_site {
            // Expression position: the output must be ONE expression. It is
            // parsed as the statement `(<output>);` — the grouping makes any
            // expression a well-formed statement without changing it.
            let wrapped = format!("({});", stamped.as_deref().unwrap_or(raw));
            let (parsed, parsed_text) = match parse_generated(&wrapped) {
                Ok(parsed) => parsed,
                Err(message) => {
                    self.diagnostics.push(Error {
                        note: None,
                        span: site,
                        msg: format!(
                            "{label} generated invalid vilan ({message}) — the \
                             output was: {}",
                            preview(&wrapped)
                        ),
                    });
                    self.output.failed_sites.push(site_key);
                    return;
                }
            };
            let [only] = parsed.as_slice() else {
                self.diagnostics.push(Error {
                    note: None,
                    span: site,
                    msg: format!(
                        "{label} must generate a single expression here (it is \
                         spliced in expression position)"
                    ),
                });
                self.output.failed_sites.push(site_key);
                return;
            };
            let (generated_blocks, _) = macro_blocks(parsed);
            if !generated_blocks.is_empty() {
                self.diagnostics.push(Error {
                    note: None,
                    span: site,
                    msg: format!(
                        "{label} generated a `macro {{ .. }}` block — macros cannot \
                         define macros (macro-engine.md §3)"
                    ),
                });
                self.output.failed_sites.push(site_key);
                return;
            }
            self.output.expressions.push((site_key, only));
            // The spliced expression may itself contain invocations.
            self.sweep_expressions(only, parsed_text, depth + 1);
        } else {
            let parse_result = match &stamped {
                None => parse_cached(raw),
                Some(stamped) => parse_generated(stamped),
            };
            let (parsed, parsed_text) = match parse_result {
                Ok(parsed) => parsed,
                Err(message) => {
                    self.diagnostics.push(Error {
                        note: None,
                        span: site,
                        msg: format!(
                            "{label} generated invalid vilan ({message}) — the \
                             output was: {}",
                            preview(stamped.as_deref().unwrap_or(raw))
                        ),
                    });
                    return;
                }
            };
            if !macro_funs(parsed).is_empty() {
                self.diagnostics.push(Error {
                    note: None,
                    span: site,
                    msg: format!(
                        "{label} generated a `macro fun` — macros cannot define \
                         macros (macro-engine.md §3)"
                    ),
                });
                return;
            }
            let (generated_blocks, _) = macro_blocks(parsed);
            if !generated_blocks.is_empty() {
                self.diagnostics.push(Error {
                    note: None,
                    span: site,
                    msg: format!(
                        "{label} generated a `macro {{ .. }}` block — macros cannot \
                         define macros (macro-engine.md §3)"
                    ),
                });
                return;
            }
            self.output.items.push((site, parsed));
            // The generated code may carry derives, services, and further
            // macro uses — the unified item scan handles them all.
            self.expand_list(parsed, parsed_text, depth + 1);
        }
    }
}

/// Runs one macro through the process-global expansion cache: key = (world,
/// macro, item source, argument sources) — §6, sound because the interpreter
/// is deterministic by construction.
fn cached_run(
    world: &World,
    entry: &str,
    name: &str,
    item_text: &str,
    arguments: &[&str],
    call_arguments: &[js::Node<'static>],
    fuel: u64,
) -> Result<&'static str, String> {
    static EXPANSIONS: OnceLock<Mutex<HashMap<u64, &'static str>>> = OnceLock::new();
    let key = {
        let mut hasher = DefaultHasher::new();
        world.key.hash(&mut hasher);
        name.hash(&mut hasher);
        item_text.hash(&mut hasher);
        arguments.hash(&mut hasher);
        hasher.finish()
    };
    let expansions = EXPANSIONS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(raw) = expansions.lock().unwrap().get(&key).copied() {
        return Ok(raw);
    }
    let source = interpreter::run_entry(
        &world.program,
        entry,
        call_arguments,
        Limits {
            fuel,
            ..Limits::default()
        },
    )
    .map_err(|failure| failure.message)?;
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    expansions.lock().unwrap().insert(key, leaked);
    Ok(leaked)
}

/// Replaces whole-identifier `__m<digits>` gensym placeholders (`meta::fresh`'s
/// outputs, §7) with per-site unique names. `None` when the text has none.
fn stamp_gensyms(text: &str, site: u32) -> Option<String> {
    if !text.contains("__m") {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 16);
    let mut index = 0;
    let mut stamped = false;
    while index < bytes.len() {
        let rest = &text[index..];
        let is_boundary =
            index == 0 || !(bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_');
        if is_boundary && rest.starts_with("__m") {
            let digits_len = rest[3..]
                .bytes()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            let next = index + 3 + digits_len;
            let ends_cleanly = next >= bytes.len()
                || !(bytes[next].is_ascii_alphanumeric() || bytes[next] == b'_');
            if digits_len > 0 && ends_cleanly {
                out.push_str("__s");
                out.push_str(&site.to_string());
                out.push_str(&rest[1..3 + digits_len]); // "_m<digits>"
                index = next;
                stamped = true;
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        index += ch.len_utf8();
    }
    stamped.then_some(out)
}

/// A one-line, length-capped rendering of generated text for error messages.
fn preview(text: &str) -> String {
    let flat = text.replace('\n', " ");
    let flat = flat.trim();
    if flat.len() > 120 {
        format!("`{}…`", &flat[..120])
    } else {
        format!("`{flat}`")
    }
}

fn slice<'a>(text: &'a str, span: Span) -> &'a str {
    let range = span.into_range();
    text.get(range.start.min(text.len())..range.end.min(text.len()))
        .unwrap_or("")
}

/// The content-addressed parse cache for UNSTAMPED macro output — re-analyses
/// skip both the interpreter and the parse.
fn parse_cached(text: &'static str) -> Result<(&'static NodeList<'static>, &'static str), String> {
    static PARSES: OnceLock<Mutex<HashMap<u64, (&'static NodeList<'static>, &'static str)>>> =
        OnceLock::new();
    let key = content_key(text);
    let parses = PARSES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = parses.lock().unwrap().get(&key).copied() {
        return Ok(cached);
    }
    let parsed = parse_generated(text)?;
    parses.lock().unwrap().insert(key, parsed);
    Ok(parsed)
}

/// Lexes + parses a macro's returned source. Unlike the trusted derive
/// generators, macro output is user code: errors are returned, not swallowed.
fn parse_generated(source: &str) -> Result<(&'static NodeList<'static>, &'static str), String> {
    use chumsky::prelude::*;
    let source: &'static str = Box::leak(source.to_string().into_boxed_str());
    // Fast path for clean output; the rich pipeline below runs only to name
    // what's wrong with a macro's malformed source.
    if let Some(root) = crate::parse_clean(source) {
        let leaked: &'static crate::span::Spanned<NodeList<'static>> = Box::leak(Box::new(root));
        return Ok((&leaked.0, source));
    }
    let (tokens, lex_errors) = crate::lexer::lexer().parse(source).into_output_errors();
    if let Some(error) = lex_errors.first() {
        return Err(error.to_string());
    }
    let Some(tokens) = tokens else {
        return Err("empty output".to_string());
    };
    let end = source.len();
    let (root, parse_errors) = crate::parser::parser()
        .map_with(|ast, e| (ast, e.span()))
        .parse(
            tokens
                .as_slice()
                .map((end..end).into(), |(token, span)| (token, span)),
        )
        .into_output_errors();
    if let Some(error) = parse_errors.first() {
        return Err(crate::render_parse_error(error, source));
    }
    match root {
        Some((mut root, _file_span)) => {
            // Expansion output walks like any other tree — its bare-`?`
            // marks become lift regions here (expression-lifting.md).
            crate::lift::rewrite_items(&mut root.0);
            let leaked: &'static crate::span::Spanned<NodeList<'static>> =
                Box::leak(Box::new(root));
            Ok((&leaked.0, source))
        }
        None => Err("empty output".to_string()),
    }
}

// --- Reflection literals (the meta.vl layout contract) ---
//
// `macro_std::meta`'s types are ordinary vilan values, so at the JS level a
// struct is its fields as a positional array (declaration order) and an enum
// variant is `[discriminant, ...payload]`. The constructors below build the
// interpreter's inputs in exactly that layout; `meta.vl` and this section
// change together (the end-to-end corpus test pins the pair).

fn string_literal(text: &str) -> js::Node<'static> {
    js::Node::String(std::borrow::Cow::Owned(text.to_string()))
}

fn array(items: Vec<js::Node<'static>>) -> js::Node<'static> {
    js::Node::Array(items)
}

fn discriminant(index: usize) -> js::Node<'static> {
    js::Node::Number(index.to_string(), None)
}

/// `TypeExpr { name, arguments }` from a written type. Named heads keep their
/// arguments; every other type form (tuples, closures, references, mapped
/// types) becomes its source text as the name with no arguments — renderable
/// back verbatim, which is all v1's syntactic macros need.
fn construct_type_expr(node: &Spanned<Node>, text: &str) -> js::Node<'static> {
    match &node.0 {
        Node::Accessor(name) => array(vec![string_literal(name), array(Vec::new())]),
        Node::AccessorWithGenerics(name, arguments) => array(vec![
            string_literal(name),
            array(
                arguments
                    .0
                    .iter()
                    .map(|argument| construct_type_expr(argument, text))
                    .collect(),
            ),
        ]),
        _ => array(vec![string_literal(slice(text, node.1)), array(Vec::new())]),
    }
}

fn void_type_expr() -> js::Node<'static> {
    array(vec![string_literal("void"), array(Vec::new())])
}

/// The `Item` value for the annotated node: `[0, StructItem]`, `[1, EnumItem]`,
/// or `[2, FunctionItem]` — the variant order declared in `meta.vl`.
fn construct_item(item: &Spanned<Node>, text: &str) -> js::Node<'static> {
    match &item.0 {
        Node::Struct(name, _generics, _external, fields) => {
            let fields = fields
                .iter()
                .flat_map(|fields| &fields.0)
                .map(|(field, _)| {
                    let (field_name, field_type, exposed) = field;
                    array(vec![
                        string_literal(field_name.0),
                        field_type
                            .as_ref()
                            .map(|type_| construct_type_expr(type_, text))
                            .unwrap_or_else(void_type_expr),
                        js::Node::Bool(*exposed),
                    ])
                })
                .collect();
            array(vec![
                discriminant(0),
                array(vec![string_literal(name.0), array(fields)]),
            ])
        }
        Node::Enum(name, _generics, variants) => {
            let variants = variants
                .0
                .iter()
                .map(|(variant, _)| {
                    let (variant_name, payload, _discriminant) = variant;
                    array(vec![
                        string_literal(variant_name),
                        array(
                            payload
                                .iter()
                                .map(|type_| construct_type_expr(type_, text))
                                .collect(),
                        ),
                    ])
                })
                .collect();
            array(vec![
                discriminant(1),
                array(vec![string_literal(name.0), array(variants)]),
            ])
        }
        Node::Func(function) => array(vec![
            discriminant(2),
            construct_function_item(function, text),
        ]),
        // The parser only puts structs/enums/functions under an attribute.
        _ => array(vec![
            discriminant(0),
            array(vec![string_literal(""), array(Vec::new())]),
        ]),
    }
}

/// A `FunctionItem` value: name, parameters (as never-exposed `Field`s, `self`
/// included — consumers skip it by name), and the written return type
/// (`void` when omitted).
fn construct_function_item(function: &Func, text: &str) -> js::Node<'static> {
    let parameters = function
        .parameters
        .0
        .iter()
        .map(|(pattern, type_, _convention, span)| {
            let parameter_name = match pattern {
                Pattern::Binding(name, _) => (*name).to_string(),
                _ => slice(text, *span).to_string(),
            };
            array(vec![
                string_literal(&parameter_name),
                type_
                    .as_ref()
                    .map(|type_| construct_type_expr(type_, text))
                    .unwrap_or_else(void_type_expr),
                js::Node::Bool(false),
            ])
        })
        .collect();
    array(vec![
        string_literal(function.name.0),
        array(parameters),
        function
            .return_type
            .as_ref()
            .map(|type_| construct_type_expr(type_, text))
            .unwrap_or_else(void_type_expr),
    ])
}

/// The `Item::Service` value for a `[service(..)]` struct — the compiler
/// gathers the same-module `[rpc]` surface (module-wide reflection stays
/// future; a service's subject INCLUDES its rpc surface by the feature's own
/// definition). Returns the literal plus the canonical INPUT text the
/// expansion cache keys on: the output depends on the sibling impls, so the
/// struct's own text alone would go stale when a method changes.
pub(crate) fn construct_service(
    client_name: Option<&str>,
    item: &Spanned<Node>,
    nodes: &NodeList,
    text: &str,
) -> Option<(js::Node<'static>, String)> {
    let Node::Struct(name, _generics, _external, Some(fields)) = &item.0 else {
        return None;
    };
    let service_name = name.0;
    let client = client_name
        .map(str::to_string)
        .unwrap_or_else(|| format!("{service_name}Client"));
    let field_values = fields
        .0
        .iter()
        .map(|(field, _)| {
            let (field_name, field_type, exposed) = field;
            array(vec![
                string_literal(field_name.0),
                field_type
                    .as_ref()
                    .map(|type_| construct_type_expr(type_, text))
                    .unwrap_or_else(void_type_expr),
                js::Node::Bool(*exposed),
            ])
        })
        .collect();
    let mut methods = Vec::new();
    let mut input = String::new();
    input.push_str(slice(text, item.1));
    input.push('\u{0}');
    input.push_str(&client);
    for (node, _span) in nodes {
        let Node::Impl(subject, impl_traits, body) = node else {
            continue;
        };
        if !impl_traits.is_empty() {
            continue;
        }
        let Node::Accessor(subject_name) = &subject.0 else {
            continue;
        };
        if *subject_name != service_name {
            continue;
        }
        for (member, member_span) in &body.0 {
            let Node::Func(function) = member else {
                continue;
            };
            if !function.rpc {
                continue;
            }
            methods.push(construct_function_item(function, text));
            input.push('\u{0}');
            input.push_str(slice(text, *member_span));
        }
    }
    let literal = array(vec![
        discriminant(3),
        array(vec![
            string_literal(service_name),
            string_literal(&client),
            array(field_values),
            array(methods),
        ]),
    ]);
    Some((literal, input))
}

/// `Arguments { values }` — the invocation's argument source texts.
fn construct_arguments(arguments: &[&str]) -> js::Node<'static> {
    array(vec![array(
        arguments
            .iter()
            .map(|argument| string_literal(argument.trim()))
            .collect(),
    )])
}
