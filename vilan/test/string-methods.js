function b/*is_empty*/(c) {
	return c.length === 0;
}
const a/*s*/ = "Hello, World";
console.log(a/*s*/.length);
console.log(a/*s*/.includes("World"));
console.log(a/*s*/.startsWith("Hello"));
console.log(a/*s*/.endsWith("!"));
console.log(a/*s*/.toUpperCase());
console.log(a/*s*/.replaceAll("o", "0"));
console.log(a/*s*/.substring(0, 5));
console.log("ab".repeat(3));
console.log(b/*is_empty*/("  hi  ".trim()));
console.log(b/*is_empty*/(""));
for (const d/*part*/ of "a,b,c".split(",")) {
	console.log(d/*part*/);
}
