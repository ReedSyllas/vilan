function eq2(self2, b2) {
	const $a = [ self2, b2 ];
	let $b = null;
	if ($a[0][0] === 0 && $a[1][0] === 0) {
		const x = $a[0][1];
		const y = $a[1][1];
		$b = x === y;
	} else if ($a[0][0] === 1 && $a[1][0] === 1) {
		$b = true;
	} else {
		$b = false;
	}
	return $b;
}
function eq3(self3, b3) {
	const $c = [ self3, b3 ];
	let $d = null;
	if ($c[0][0] === 0 && $c[1][0] === 0) {
		const x2 = $c[0][1];
		const y2 = $c[1][1];
		$d = x2 === y2;
	} else if ($c[0][0] === 1 && $c[1][0] === 1) {
		const x3 = $c[0][1];
		const y3 = $c[1][1];
		$d = x3 === y3;
	} else {
		$d = false;
	}
	return $d;
}
function eq(self, b) {
	return self[0] === b[0] && self[1] === b[1];
}
const p1 = [ 1, 2 ];
const p2 = [ 1, 2 ];
const p3 = [ 3, 4 ];
console.log(eq(p1, p2));
console.log(eq(p1, p3));
console.log(!(eq(p1, p3)));
const a = [ 0, 5 ];
console.log(eq2(a, [ 0, 5 ]));
console.log(eq2(a, [ 1 ]));
console.log(!(eq2(a, [ 0, 7 ])));
const r = [ 0, 1 ];
console.log(eq3(r, [ 0, 1 ]));
console.log(eq3(r, [ 1, "x" ]));
console.log(5 === 5);
console.log("a" === "b");
console.log(true === true);
