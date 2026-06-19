function b(c) {
	return c[0];
}
function d(e) {
	return [ 0, e[0] ];
}
const a/*b*/ = [ [ 5 ] ];
console.log(b(a/*b*/)[0]);
const f = d(a/*b*/);
let g = null;
if (f[0] === 0) {
	const h/*n*/ = f[1];
	g = console.log(h/*n*/[0]);
} else {
	g = console.log(-(1));
}
process.exit(g);
