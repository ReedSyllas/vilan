function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
let a = [ 0, 0 ];
let total = 0;
for (const n of [ 1, 2, 3 ]) {
	let b = __clone(a);
	b[0] = b[0] + 1;
	total = total + b[0];
}
console.log(total);
