function d/*add*/(e, f) {
	return [ e[0] + f[0], e[1] + f[1] ];
}
function h/*mul*/(i, j) {
	return [ i[0] * j[0], i[1] * j[1] ];
}
const a/*a*/ = [ 1, 2 ];
const b/*b*/ = [ 3, 4 ];
const c/*sum*/ = d/*add*/(a/*a*/, b/*b*/);
console.log(c/*sum*/[0]);
console.log(c/*sum*/[1]);
const g/*product*/ = h/*mul*/(a/*a*/, b/*b*/);
console.log(g/*product*/[0]);
console.log(g/*product*/[1]);
