function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
let a/*a*/ = [ 0, 0 ];
let b/*total*/ = 0;
for (const c/*n*/ of [ 1, 2, 3 ]) {
	let d/*b*/ = __clone(a/*a*/);
	d/*b*/[0] = d/*b*/[0] + 1;
	b/*total*/ = b/*total*/ + d/*b*/[0];
}
console.log(b/*total*/);
