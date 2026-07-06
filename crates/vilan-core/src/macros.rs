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
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::Error;
use crate::interpreter::{self, Limits};
use crate::node::{Func, ImportBranch, Node, NodeList, Pattern};
use crate::options::BuildOptions;
use crate::span::{Span, Spanned};
use crate::transformer::{JsProgram, js, transform_functions};
use crate::{PackageSpec, Platform, Workspace, analyze_source};

/// Expansion depth cap: a macro's output may carry further attribute
/// invocations; past this the chain is reported instead of chased.
const MAX_DEPTH: u32 = 16;

/// A compiled per-file macro world: the transformed program the interpreter
/// executes, plus each macro's emitted entry name and parameter count.
pub(crate) struct World {
    key: u64,
    program: JsProgram<'static>,
    entries: HashMap<String, (String, Option<MacroShape>)>,
}

#[derive(Default)]
pub(crate) struct MacroRegistry {
    by_name: HashMap<String, MacroDef>,
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
    world: Arc<World>,
    entry: String,
    /// `None` = a HELPER (§3: helpers are macro funs) — compiled into the
    /// world and callable from other macros, but not dispatchable from
    /// program code (its signature is not a macro shape, or it doesn't
    /// return `Source`).
    shape: Option<MacroShape>,
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

/// Registers a file's `macro fun`s: checks their bodies' hermeticity, compiles
/// the file's macro world (cached by blanked content), and binds each macro
/// name. Diagnostics carry spans into THIS file (the caller attributes them).
pub(crate) fn register_file(
    registry: &mut MacroRegistry,
    nodes: &NodeList,
    text: &str,
    file: &Path,
    std: &PackageSpec,
    builtins: Option<&MacroRegistry>,
    diagnostics: &mut Vec<Error>,
) {
    let definitions = macro_funs(nodes);
    if definitions.is_empty() {
        return;
    }
    let Some(macro_std) = resolve_macro_std(std) else {
        diagnostics.push(Error {
            span: definitions[0].1,
            msg: "the `macro_std` package was not found beside `std` — macros need the \
                  toolchain's `macro_std`"
                .to_string(),
        });
        return;
    };

    // Hermeticity (§4): a macro body imports only from `macro_std`. Checked on
    // the ORIGINAL parse so the error points at the offending import.
    let mut hermetic = true;
    for (function, _) in &definitions {
        if let Some(body) = &function.body {
            for statement in body.0.0.iter().chain(std::iter::once(&*body.0.1)) {
                check_hermetic_imports(statement, diagnostics, &mut hermetic);
            }
        }
    }
    if !hermetic {
        return;
    }

    let blanked = blank_to_world(text, &definitions);
    let world = match compile_world(blanked, file, std, &macro_std, &definitions) {
        Ok(world) => world,
        Err(errors) => {
            diagnostics.extend(errors);
            return;
        }
    };
    for (function, _) in &definitions {
        let name = function.name.0.to_string();
        if registry.by_name.contains_key(&name) {
            diagnostics.push(Error {
                span: function.name.1,
                msg: format!("a macro named `{name}` is already defined"),
            });
            continue;
        }
        // Built-in derive names are reserved: a user macro shadowing
        // `PartialEq` would fire twice (the builtin channel AND the user
        // registry). The builtins file itself registers with `builtins: None`.
        if let Some(builtins) = builtins {
            if builtins.by_name.contains_key(&name) {
                diagnostics.push(Error {
                    span: function.name.1,
                    msg: format!(
                        "`{name}` is a built-in derive — a user macro cannot take its name"
                    ),
                });
                continue;
            }
        }
        let Some((entry, shape)) = world.entries.get(&name).cloned() else {
            continue; // the world compile reported why
        };
        registry.by_name.insert(
            name,
            MacroDef {
                world: world.clone(),
                entry,
                shape,
            },
        );
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
fn blank_to_world(text: &str, definitions: &[(&Func, Span)]) -> String {
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
    String::from_utf8(blanked).expect("blanking preserves UTF-8 (multibyte bytes become spaces)")
}

fn content_key(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn compile_world(
    blanked: String,
    file: &Path,
    std: &PackageSpec,
    macro_std: &PackageSpec,
    definitions: &[(&Func, Span)],
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
    };
    let (program, errors) = analyze_source(
        leaked,
        std,
        file.parent().unwrap_or(Path::new(".")),
        file,
        Some(Platform::default()),
        &workspace,
    );
    if !errors.is_empty() {
        return Err(errors
            .into_iter()
            .map(|error| Error {
                span: error.span,
                msg: format!("in this macro: {}", error.msg),
            })
            .collect());
    }
    let Some(program) = program else {
        return Err(vec![Error {
            span: definitions[0].1,
            msg: "the macro world failed to compile".to_string(),
        }]);
    };
    // The world outlives this analysis (it's cached process-globally), so the
    // program is leaked — bounded: one leak per distinct macro-definition set,
    // the same shape as the parse cache's leaks.
    let program: &'static crate::Program<'static> = Box::leak(Box::new(program));

    let mut roots = Vec::new();
    let mut root_names = Vec::new();
    let mut errors = Vec::new();
    for (function, span) in definitions {
        let name = function.name.0;
        let id = program
            .scopes
            .get(&program.global_scope_id)
            .and_then(|scope| scope.name_to_id_map.get(name))
            .copied();
        match id {
            Some(id) if program.functions.contains_key(&id) => {
                // A non-macro signature (or a non-`Source` return) is a HELPER:
                // compiled and callable inside the world, never dispatched.
                roots.push(id);
                root_names.push((name.to_string(), macro_shape(function)));
            }
            _ => errors.push(Error {
                span: *span,
                msg: format!("the macro `{name}` did not compile to a callable function"),
            }),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let (js_program, emitted) = transform_functions(program, &BuildOptions::default(), &roots)
        .map_err(|error| vec![error])?;
    let entries = roots
        .iter()
        .zip(root_names)
        .map(|(id, (name, shape))| (name, (emitted.get(id).cloned().unwrap_or_default(), shape)))
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
    /// Generated ITEM lists — walked after the originating file's items.
    pub(crate) items: Vec<&'static NodeList<'static>>,
    /// Expression splices: invocation node address → the replacement
    /// expression the walk substitutes.
    pub(crate) expressions: Vec<(usize, &'static Spanned<Node<'static>>)>,
    /// Item-position invocation node addresses (they walk to nothing).
    pub(crate) item_sites: Vec<usize>,
    /// Expression sites whose expansion FAILED — the failure is already a
    /// diagnostic; the walk substitutes an error entity without piling on.
    pub(crate) failed_sites: Vec<usize>,
}

struct Expander<'r, 'd> {
    registry: &'r MacroRegistry,
    std: &'r PackageSpec,
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
    registry: &MacroRegistry,
    std: &PackageSpec,
    nodes: &NodeList,
    text: &str,
    diagnostics: &mut Vec<Error>,
    site_counter: &mut u32,
    depth: u32,
) -> ExpansionOutput {
    let mut expander = Expander {
        registry,
        std,
        diagnostics,
        site_counter,
        output: ExpansionOutput::default(),
    };
    expander.expand_list(nodes, text, depth);
    expander.output
}

impl Expander<'_, '_> {
    fn expand_list(&mut self, nodes: &NodeList, text: &str, depth: u32) {
        for node in nodes {
            self.expand_item_position(node, text, depth);
        }
    }

    /// A node in ITEM position: attributes, item invocations, derives — and a
    /// sweep of everything else for expression-position invocations.
    fn expand_item_position(&mut self, node: &Spanned<Node>, text: &str, depth: u32) {
        match &node.0 {
            Node::Export(inner) => self.expand_item_position(inner, text, depth),
            // `mod` bodies are item position too.
            Node::Module(_, body) => {
                for child in &body.0 {
                    self.expand_item_position(child, text, depth);
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
            // `[derive(Name)]` with a registered name dispatches like an
            // attribute with no arguments; built-in names keep their Rust
            // generators; unknown names keep today's behavior.
            Node::Derive(names, item) => {
                for name in names.iter() {
                    if self.registry.by_name.contains_key(*name) {
                        self.run_attribute(name, node.1, item, &[], text, depth);
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
            // Everything else: hunt expression-position invocations inside.
            _ => self.sweep_expressions(node, text, depth),
        }
    }

    /// Finds `macro name(..)` in EXPRESSION position anywhere under `node`
    /// (macro definitions excluded — their bodies belong to the world).
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
            _ => {
                node.0
                    .for_each_child(&mut |child| self.sweep_expressions(child, text, depth));
            }
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
        let Some(def) = self.registry.by_name.get(name) else {
            self.diagnostics.push(Error {
                span: site,
                msg: format!("no macro named `{name}` is in scope"),
            });
            return;
        };
        let call_arguments = match def.shape {
            None => {
                self.diagnostics.push(Error {
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
        let Some(def) = self.registry.by_name.get(name) else {
            self.diagnostics.push(Error {
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
        if depth >= MAX_DEPTH {
            self.diagnostics.push(Error {
                span: site,
                msg: format!(
                    "macro expansion did not settle after {MAX_DEPTH} rounds — the chain ends \
                     at `{name}`"
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
        let raw: &'static str = match cached_run(def, name, item_text, arguments, &call_arguments) {
            Ok(raw) => raw,
            Err(message) => {
                self.diagnostics.push(Error {
                    span: site,
                    msg: format!("macro `{name}` failed at expansion time: {message}"),
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
                        span: site,
                        msg: format!(
                            "macro `{name}` generated invalid vilan ({message}) — the \
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
                    span: site,
                    msg: format!(
                        "macro `{name}` must generate a single expression here (it is \
                         spliced in expression position)"
                    ),
                });
                self.output.failed_sites.push(site_key);
                return;
            };
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
                        span: site,
                        msg: format!(
                            "macro `{name}` generated invalid vilan ({message}) — the \
                             output was: {}",
                            preview(stamped.as_deref().unwrap_or(raw))
                        ),
                    });
                    return;
                }
            };
            if !macro_funs(parsed).is_empty() {
                self.diagnostics.push(Error {
                    span: site,
                    msg: format!(
                        "macro `{name}` generated a `macro fun` — macros cannot define \
                         macros (macro-engine.md §3)"
                    ),
                });
                return;
            }
            self.output.items.push(parsed);
            // The generated code may carry built-in derives and further uses.
            if let Some(derived) =
                crate::analyzer::expand_derives(parsed, parsed_text, self.std, self.diagnostics)
            {
                self.output.items.push(derived);
            }
            self.expand_list(parsed, parsed_text, depth + 1);
        }
    }
}

/// Runs one macro through the process-global expansion cache: key = (world,
/// macro, item source, argument sources) — §6, sound because the interpreter
/// is deterministic by construction.
fn cached_run(
    def: &MacroDef,
    name: &str,
    item_text: &str,
    arguments: &[&str],
    call_arguments: &[js::Node<'static>],
) -> Result<&'static str, String> {
    static EXPANSIONS: OnceLock<Mutex<HashMap<u64, &'static str>>> = OnceLock::new();
    let key = {
        let mut hasher = DefaultHasher::new();
        def.world.key.hash(&mut hasher);
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
        &def.world.program,
        &def.entry,
        call_arguments,
        Limits::default(),
    )
    .map_err(|failure| failure.message)?;
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    expansions.lock().unwrap().insert(key, leaked);
    Ok(leaked)
}

/// The toolchain's BUILT-IN derive macros (`<std package dir>/derives.vl` —
/// deliberately outside the layer roots, so it can never load as a module):
/// compiled once per std path and consulted by `expand_derives` before the
/// Rust generators. This is Phase 3's migration seam (§10): a derive moves to
/// user-land vilan by gaining a `macro fun` here whose output is byte-
/// identical to the Rust generator it replaces; the corpus goldens referee.
pub(crate) fn builtin_derives(std: &PackageSpec) -> &'static (MacroRegistry, Vec<Error>) {
    static BUILTINS: OnceLock<
        Mutex<HashMap<std::path::PathBuf, &'static (MacroRegistry, Vec<Error>)>>,
    > = OnceLock::new();
    let by_root = BUILTINS.get_or_init(|| Mutex::new(HashMap::new()));
    let root = std.base_root.clone();
    if let Some(cached) = by_root.lock().unwrap().get(&root) {
        return cached;
    }
    // Compiling derives.vl's macro world runs a nested `analyze`, whose own
    // `expand_derives` re-enters here. Seed an EMPTY registry first so the
    // nested lookup terminates — inside the macro world the Rust generators
    // (byte-identical by the migration contract) serve any std derive.
    let placeholder: &'static (MacroRegistry, Vec<Error>) =
        Box::leak(Box::new((MacroRegistry::default(), Vec::new())));
    by_root.lock().unwrap().insert(root.clone(), placeholder);
    let mut registry = MacroRegistry::default();
    let mut diagnostics = Vec::new();
    let path = std
        .base_root
        .parent()
        .map(|parent| parent.join("derives.vl"));
    if let Some(path) = path.filter(|path| path.is_file()) {
        match crate::analyzer::load_package_module(&path.to_string_lossy()) {
            Some(loaded) => {
                for error in loaded.parse_errors {
                    diagnostics.push(Error {
                        span: (0..0).into(),
                        msg: format!("in the built-in derives ({}): {error}", path.display()),
                    });
                }
                register_file(
                    &mut registry,
                    &loaded.ast.0,
                    loaded.text,
                    &path,
                    std,
                    None,
                    &mut diagnostics,
                );
            }
            None => diagnostics.push(Error {
                span: (0..0).into(),
                msg: format!(
                    "the built-in derives file is unreadable: {}",
                    path.display()
                ),
            }),
        }
    }
    let leaked: &'static (MacroRegistry, Vec<Error>) = Box::leak(Box::new((registry, diagnostics)));
    by_root.lock().unwrap().insert(root, leaked);
    leaked
}

/// Runs a built-in derive against an item, returning the generated source
/// text (uncached failures are the caller's diagnostics; output rides the
/// same expansion cache as user macros).
pub(crate) fn run_builtin_derive(
    def: &MacroDef,
    name: &str,
    item: &Spanned<Node>,
    text: &str,
) -> Result<&'static str, String> {
    let call_arguments = vec![construct_item(item, text)];
    cached_run(def, name, slice(text, item.1), &[], &call_arguments)
}

/// Looks up a built-in derive by name.
pub(crate) fn builtin_derive<'r>(builtins: &'r MacroRegistry, name: &str) -> Option<&'r MacroDef> {
    builtins.by_name.get(name)
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
        return Err(error.to_string());
    }
    match root {
        Some((root, _file_span)) => {
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
                    let (field_name, field_type, _exposed) = field;
                    array(vec![
                        string_literal(field_name.0),
                        field_type
                            .as_ref()
                            .map(|type_| construct_type_expr(type_, text))
                            .unwrap_or_else(void_type_expr),
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
        Node::Func(function) => {
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
                    ])
                })
                .collect();
            array(vec![
                discriminant(2),
                array(vec![
                    string_literal(function.name.0),
                    array(parameters),
                    function
                        .return_type
                        .as_ref()
                        .map(|type_| construct_type_expr(type_, text))
                        .unwrap_or_else(void_type_expr),
                ]),
            ])
        }
        // The parser only puts structs/enums/functions under an attribute.
        _ => array(vec![
            discriminant(0),
            array(vec![string_literal(""), array(Vec::new())]),
        ]),
    }
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
