function d/*scale*/(e, f) {
	e[0][e[1]] = e[0][e[1]] * f;
}
let a/*a*/ = [ 10 ];
const b/*c*/ = [ a/*a*/, 0 ];
b/*c*/[0][b/*c*/[1]] = 40;
console.log(a/*a*/[0]);
let c/*n*/ = [ 5 ];
d/*scale*/([ c/*n*/, 0 ], 4);
console.log(c/*n*/[0]);
