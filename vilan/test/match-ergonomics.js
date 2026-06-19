function d/*classify*/(e) {
	const f = e[0];
	let g = null;
	if (f === 0) {
		const h/*base*/ = e[1];
		g = h/*base*/;
	} else if (f === 1) {
		g = e[1] * 10;
	} else {
		const i/*other*/ = f;
		g = i/*other*/ + e[1];
	}
	return g;
}
const a/*x*/ = 2;
const b/*y*/ = 5;
const c/*p*/ = [ a/*x*/, b/*y*/ ];
console.log(d/*classify*/(c/*p*/));
console.log(d/*classify*/([ 0, 9 ]));
console.log(d/*classify*/([ 1, 4 ]));
