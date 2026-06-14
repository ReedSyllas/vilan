function c(d, e) {
	const f = d;
	let g = null;
	if (f[0] === 0) {
		const h/*x*/ = f[1];
		g = [ 0, e(h/*x*/) ];
	} else {
		g = [ 1 ];
	}
	return g;
}
const a/*value*/ = [ 0, 5 ];
console.log(c(a/*value*/, (b) => {
	return b * 6;
}));
