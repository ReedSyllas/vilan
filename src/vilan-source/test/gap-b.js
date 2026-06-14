function b/*unzip*/(c) {
	const d = c;
	let e = null;
	if (d[0] === 0) {
		const f/*x*/ = d[1][0];
		const g/*y*/ = d[1][1];
		e = [ [ 0, f/*x*/ ], [ 0, g/*y*/ ] ];
	} else {
		e = [ [ 1 ], [ 1 ] ];
	}
	return e;
}
const a/*pair*/ = [ 0, [ 3, 7 ] ];
console.log(b/*unzip*/(a/*pair*/));
const h/*empty*/ = [ 1 ];
console.log(b/*unzip*/(h/*empty*/));
