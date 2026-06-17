function d/*combine*/(e, f) {
	return [ e[0] + f[0] ];
}
function b(c) {
	return d/*combine*/(d/*combine*/(c, c), c);
}
const a/*c*/ = [ 5 ];
console.log(b(a/*c*/)[0]);
