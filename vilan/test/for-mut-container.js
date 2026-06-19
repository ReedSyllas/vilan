function b/*next_mut*/(c) {
	let e = null;
	if (c[1] < c[0].length) {
		const d/*index*/ = c[1];
		c[1] = c[1] + 1;
		e = [ 0, [ c[0], d/*index*/ ] ];
	} else {
		e = [ 1 ];
	}
	return e;
}
let a/*counter*/ = [ [ 1, 2, 3 ], 0 ];
const f = a/*counter*/;
while (true) {
	const g = b/*next_mut*/(f);
	if (g[0] !== 0) {
		break;
	}
	const h/*element*/ = g[1];
	h/*element*/[0][h/*element*/[1]] = h/*element*/[0][h/*element*/[1]] * 10;
}
console.log(a/*counter*/[0][0]);
console.log(a/*counter*/[0][1]);
console.log(a/*counter*/[0][2]);
