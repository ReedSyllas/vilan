function f/*value_of*/(g) {
	return g[0];
}
function c(d, e) {
	return f/*value_of*/(d) + f/*value_of*/(e);
}
const a/*a*/ = [ 4 ];
const b/*b*/ = [ 6 ];
console.log(c(a/*a*/, b/*b*/));
