function j/*is_empty*/(k) {
	return b(k) === 0;
}
const a/*s*/ = "Hello, World";
console.log(b(a/*s*/));
console.log(c(a/*s*/, "World"));
console.log(d(a/*s*/, "Hello"));
console.log(e(a/*s*/, "!"));
console.log(f(a/*s*/));
console.log(g(a/*s*/, "o", "0"));
console.log(h(a/*s*/, 0, 5));
console.log(i("ab", 3));
console.log(j/*is_empty*/("  hi  ".trim()));
console.log(j/*is_empty*/(""));
for (const m/*part*/ of l("a,b,c", ",")) {
	console.log(m/*part*/);
}
