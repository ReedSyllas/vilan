function d/*scale*/(e, f) {
	e[0] = e[0] * f;
}
let a/*a*/ = [ 10 ];
const b/*c*/ = a/*a*/;
b/*c*/[0] = 40;
console.log(a/*a*/[0]);
let c/*n*/ = [ 5 ];
d/*scale*/(c/*n*/, 4);
console.log(c/*n*/[0]);
