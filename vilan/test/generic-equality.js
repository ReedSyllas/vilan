function eq(self, b) {
	return self[0] === b[0] && self[1] === b[1];
}
function $a(a, b) {
	return eq(a, b);
}
function $b(a, b) {
	return eq(a, b);
}
function $c(a, b) {
	return !(eq(a, b));
}
function $d(a, b) {
	return a === b;
}
function $e(a, b) {
	return a !== b;
}
function $f(self, b) {
	const $g = [ self, b ];
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
