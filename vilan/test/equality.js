function eq(self, b) {
	const $a = [ self, b ];
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
function eq2(self, b) {
	const $c = [ self, b ];
	let $d = null;
	if ($c[0][0] === 0 && $c[1][0] === 0) {
		const x = $c[0][1];
		const y = $c[1][1];
		$d = x === y;
	} else if ($c[0][0] === 1 && $c[1][0] === 1) {
		const x2 = $c[0][1];
		const y2 = $c[1][1];
		$d = x2 === y2;
	} else {
		$d = false;
	}
	return $d;
}
function eq3(self, b) {
	return self[0] === b[0] && self[1] === b[1];
}
const p1 = [ 1, 2 ];
const p2 = [ 1, 2 ];
const p3 = [ 3, 4 ];
console.log(eq3(p1, p2));
console.log(eq3(p1, p3));
console.log(!(eq3(p1, p3)));
const a = [ 0, 5 ];
console.log(eq(a, [ 0, 5 ]));
console.log(eq(a, [ 1 ]));
console.log(!(eq(a, [ 0, 7 ])));
const r = [ 0, 1 ];
console.log(eq2(r, [ 0, 1 ]));
console.log(eq2(r, [ 1, "x" ]));
console.log(5 === 5);
console.log("a" === "b");
console.log(true === true);
