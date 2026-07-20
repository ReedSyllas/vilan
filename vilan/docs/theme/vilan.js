// vilan syntax highlighting for the rendered docs (proposal/docs-site.md §4).
//
// Two jobs:
// 1. Register a highlight.js grammar for vilan (regex-level — keywords,
//    types, suffixed numbers, strings and i-strings, attributes, comments;
//    lexer-true highlighting is the recorded v2).
// 2. The fence-tag shim: docs fences carry harness tags the highlighter
//    doesn't know (```vilan,browser → class "language-vilan,browser").
//    Normalize those to plain vilan, then highlight. `text` fences (the
//    ASCII diagrams) and `fragment`-tagged blocks highlight as vilan too —
//    fragments are still vilan syntax, just not compilable standalone.
(function () {
	if (typeof hljs === "undefined") {
		return;
	}

	hljs.registerLanguage("vilan", function (hljs) {
		const KEYWORDS = {
			keyword:
				"async await borrows const else enum export external for fun if impl " +
				"import in is jump let macro match mod mut own resource ret struct trait " +
				"type use with",
			literal: "true false null void self Self",
			built_in: "print panic assert",
		};
		const NUMBER = {
			className: "number",
			variants: [
				{ begin: "\\b0x[0-9a-fA-F]+(?:[iu](?:8|16|32|53)|f32|f64|[fn])?" },
				{ begin: "\\b\\d+(?:\\.\\d+)?(?:[iu](?:8|16|32|53)|f32|f64|[fn])?" },
			],
		};
		const PLAIN_STRING = {
			className: "string",
			begin: '"',
			end: '"',
			contains: [{ begin: "\\\\." }],
		};
		// i"…{hole}…" — the holes carry expressions; render them as substitutions.
		const INTERPOLATED = {
			className: "string",
			begin: 'i"',
			end: '"',
			contains: [
				{ begin: "\\\\." },
				{ className: "subst", begin: "\\{", end: "\\}" },
			],
		};
		const MULTILINE = {
			className: "string",
			begin: '"""',
			end: '"""',
		};
		const ATTRIBUTE = {
			className: "meta",
			begin: "^\\s*\\[(?:derive|service|extern|must_use|rpc|trait_only|doc|expose|macro)\\b",
			end: "\\]",
		};
		const TYPE = {
			className: "type",
			begin: "\\b[A-Z][a-zA-Z0-9_]*",
		};
		const FUNCTION = {
			className: "title",
			begin: "(?<=\\bfun\\s)[a-z_][a-zA-Z0-9_]*",
		};
		return {
			name: "vilan",
			keywords: KEYWORDS,
			contains: [
				hljs.COMMENT("//", "$"),
				ATTRIBUTE,
				MULTILINE,
				INTERPOLATED,
				PLAIN_STRING,
				NUMBER,
				FUNCTION,
				TYPE,
			],
		};
	});

	// The shim: mdBook already ran highlight.js over the page before
	// additional-js loads, so re-highlight every vilan-tagged block with the
	// grammar registered above. (mdBook bundles highlight.js v10, whose
	// entry point is highlightBlock; prefer highlightElement when a newer
	// bundle provides it.)
	var highlight = hljs.highlightElement || hljs.highlightBlock;
	document.querySelectorAll("code[class*='language-vilan']").forEach(function (block) {
		block.className = "language-vilan";
		highlight.call(hljs, block);
	});
})();
