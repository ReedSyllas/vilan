function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	return value;
}
function b(c) {
	return c.size === 0;
}
let a/*numbers*/ = new Set();
a/*numbers*/.add(1);
a/*numbers*/.add(2);
a/*numbers*/.add(2);
a/*numbers*/.add(3);
console.log(a/*numbers*/.size);
console.log(a/*numbers*/.has(2));
console.log(a/*numbers*/.has(9));
a/*numbers*/.delete(2);
console.log(a/*numbers*/.has(2));
console.log(a/*numbers*/.size);
console.log(b(a/*numbers*/));
let d/*total*/ = 0;
for (const e/*value*/ of a/*numbers*/) {
	d/*total*/ = d/*total*/ + e/*value*/;
}
console.log(d/*total*/);
let f/*copy*/ = __clone(a/*numbers*/);
f/*copy*/.add(100);
console.log(a/*numbers*/.has(100));
console.log(f/*copy*/.has(100));
let g/*words*/ = new Set();
g/*words*/.add("hi");
g/*words*/.add("hi");
g/*words*/.add("bye");
console.log(g/*words*/.size);
console.log(g/*words*/.has("hi"));
let h/*empty*/ = new Set();
console.log(b(h/*empty*/));
