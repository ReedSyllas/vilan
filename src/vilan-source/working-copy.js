function c/*map*/(d, e) {
	const f = d;
	let g = null;
	if (f[0] === 0) {
		const h/*x*/ = f[1];
		g = e(h/*x*/);
	} else {
		g = [ 1 ];
	}
	return g;
}
const a/*value*/ = [ 0, 5 ];
console.log(c/*map*/(a/*value*/, (b) => {
	return b * 6;
}));
