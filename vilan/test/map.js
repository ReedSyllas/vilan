function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function __map_get(map, key) {
	return map.has(key) ? [ 0, __clone(map.get(key)) ] : [ 1 ];
}
function b(c, d) {
	const e = c;
	let f = null;
	if (e[0] === 0) {
		const g/*x*/ = e[1];
		f = g/*x*/;
	} else {
		f = d;
	}
	return f;
}
function h(i) {
	const j = i;
	return j[0] === 0;
}
function k(l) {
	return l.size === 0;
}
let a/*scores*/ = new Map();
a/*scores*/.set("alice", 1);
a/*scores*/.set("bob", 2);
a/*scores*/.set("carol", 3);
console.log(a/*scores*/.size);
console.log(a/*scores*/.has("bob"));
console.log(a/*scores*/.has("dave"));
console.log(b(__map_get(a/*scores*/, "bob"), 0));
console.log(b(__map_get(a/*scores*/, "dave"), -(1)));
console.log(h(__map_get(a/*scores*/, "alice")));
a/*scores*/.set("bob", 22);
console.log(b(__map_get(a/*scores*/, "bob"), 0));
console.log(a/*scores*/.size);
a/*scores*/.delete("bob");
console.log(a/*scores*/.has("bob"));
console.log(a/*scores*/.size);
console.log(k(a/*scores*/));
let m/*copy*/ = __clone(a/*scores*/);
m/*copy*/.set("dave", 4);
console.log(a/*scores*/.has("dave"));
console.log(m/*copy*/.has("dave"));
let n/*names*/ = new Map();
n/*names*/.set(1, "one");
n/*names*/.set(2, "two");
console.log(b(__map_get(n/*names*/, 1), "?"));
console.log(b(__map_get(n/*names*/, 9), "?"));
let o/*empty*/ = new Map();
console.log(k(o/*empty*/));
