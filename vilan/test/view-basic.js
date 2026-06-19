function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
let c = [ 10 ];
const v = c;
v[0] = 99;
console.log(c[0]);
let e = [ 10 ];
let d = __clone(e);
d[0] = 1;
console.log(e[0]);
