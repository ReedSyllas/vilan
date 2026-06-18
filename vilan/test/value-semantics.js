function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function e/*sum*/(f) {
	let g/*total*/ = h/*default*/();
	let i/*seeded*/ = false;
	for (const j/*item*/ of f) {
		if (i/*seeded*/) {
			g/*total*/ = g/*total*/ + j/*item*/;
		} else {
			g/*total*/ = j/*item*/;
			i/*seeded*/ = true;
		}
	}
	return g/*total*/;
}
function h/*default*/() {

}
let a/*a*/ = [ 1, 2 ];
let b/*b*/ = __clone(a/*a*/);
b/*b*/[0] = 99;
console.log(a/*a*/[0]);
console.log(b/*b*/[0]);
let c/*xs*/ = [  ];
c/*xs*/.push(1);
c/*xs*/.push(2);
let d/*ys*/ = __clone(c/*xs*/);
d/*ys*/.push(99);
console.log(e/*sum*/(c/*xs*/));
console.log(e/*sum*/(d/*ys*/));
