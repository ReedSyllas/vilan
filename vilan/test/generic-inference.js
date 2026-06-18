function d/*pick*/(e, f) {
	return e + f;
}
function h/*pick*/(i, j) {
	return [ i[0] + j[0] ];
}
function a(b, c) {
	return d/*pick*/(b, c);
}
function g(b, c) {
	return h/*pick*/(b, c);
}
console.log(a(2, 3));
console.log(g([ 10 ], [ 20 ]));
