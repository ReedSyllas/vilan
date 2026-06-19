function __shared_new(value) {
	return { v: value };
}
const a/*s*/ = __shared_new([ 0 ]);
const b/*a*/ = a/*s*/;
const c/*b*/ = a/*s*/;
b/*a*/.v[0] = b/*a*/.v[0] + 1;
b/*a*/.v[0] = b/*a*/.v[0] + 1;
console.log(c/*b*/.v[0]);
console.log(a/*s*/.v[0]);
const d/*other*/ = __shared_new([ 100 ]);
d/*other*/.v[0] = 50;
console.log(d/*other*/.v[0]);
console.log(a/*s*/.v[0]);
