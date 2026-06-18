function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
let a/*c*/ = [ 10 ];
const b/*v*/ = a/*c*/;
b/*v*/[0] = 99;
console.log(a/*c*/[0]);
let c/*e*/ = [ 10 ];
let d/*d*/ = __clone(c/*e*/);
d/*d*/[0] = 1;
console.log(c/*e*/[0]);
