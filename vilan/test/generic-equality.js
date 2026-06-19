function eq(self, b2) {
	return self[0] === b2[0] && self[1] === b2[1];
}
function $a(a, b) {
	return eq(a, b);
}
function $b(a2, b3) {
	return eq(a2, b3);
}
function $c(a3, b4) {
	return !(eq(a3, b4));
}
function $d(a, b) {
	return a === b;
}
function $e(a3, b4) {
	return a3 !== b4;
}
function $f(self2, b5) {
	const $g = [ self2, b5 ];
	let $h = null;
	if ($g[0][0] === 0 && $g[1][0] === 0) {
		const x = $g[0][1];
		const y = $g[1][1];
		$h = eq(x, y);
	} else if ($g[0][0] === 1 && $g[1][0] === 1) {
		$h = true;
	} else {
		$h = false;
	}
	return $h;
}
const p1 = [ 1, 2 ];
const p2 = [ 1, 2 ];
const p3 = [ 3, 4 ];
console.log($a(p1, p2));
console.log($a(p1, p3));
console.log($b(p1, p2));
console.log($c(p1, p3));
console.log($d(5, 5));
console.log($d(5, 9));
console.log($e(5, 9));
const some_a = [ 0, p1 ];
const some_b = [ 0, p2 ];
const some_c = [ 0, p3 ];
console.log($f(some_a, some_b));
console.log($f(some_a, some_c));
console.log(!($f(some_a, some_c)));
console.log($f(some_a, [ 1 ]));
