function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function bump(c2) {
	c2[0] = c2[0] + 1;
}
function peek(c3) {
	return c3[0];
}
let c = [ 10 ];
bump(c);
console.log(c[0]);
console.log(peek(__clone(c)));
