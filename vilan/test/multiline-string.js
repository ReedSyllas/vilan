const text = "    line 1\nline 2\n\n  line 3";
console.log(text);
const raw = "no \\n escape, literal {braces}";
console.log(raw);
const code = "fun nested() {\n\tbody();\n}";
console.log(code);
const line = "key: value!";
const $a = line;
let $b = null;
if ($a === "key: value!") {
	$b = console.log("matched");
} else {
	$b = console.log("no match");
}
process.exit($b);
