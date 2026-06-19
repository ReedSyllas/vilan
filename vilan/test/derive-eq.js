function d/*eq*/(e, f) {
	return e[0] === f[0] && e[1] === f[1];
}
function i/*eq*/(j, k) {
	return true;
}
const a/*a*/ = [ 1, 2 ];
const b/*b*/ = [ 1, 2 ];
const c/*c*/ = [ 9, 2 ];
console.log(d/*eq*/(a/*a*/, b/*b*/));
console.log(d/*eq*/(a/*a*/, c/*c*/));
console.log(!(d/*eq*/(a/*a*/, c/*c*/)));
const g/*u1*/ = [  ];
const h/*u2*/ = [  ];
console.log(i/*eq*/(g/*u1*/, h/*u2*/));
