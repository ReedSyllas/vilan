function new2(start, end) {
	return [ start, end ];
}
function next(self) {
	let $a = null;
	if (self[0] < self[1]) {
		const value = self[0];
		self[0] = self[0] + 1;
		$a = [ 0, value ];
	} else {
		$a = [ 1 ];
	}
	return $a;
}
let sum = 0;
const $b = new2(1, 10);
while (true) {
	const $c = next($b);
	if ($c[0] !== 0) {
		break;
	}
	const i = $c[1];
	sum = sum + i;
}
console.log(sum);
let count = 0;
const $d = new2(0, 5);
while (true) {
	const $e = next($d);
	if ($e[0] !== 0) {
		break;
	}
	const j = $e[1];
	count = count + 1;
}
console.log(count);
let empty = 0;
const $f = new2(3, 3);
while (true) {
	const $g = next($f);
	if ($g[0] !== 0) {
		break;
	}
	const k = $g[1];
	empty = empty + 1;
}
console.log(empty);
let squares = [  ];
const $h = new2(1, 5);
while (true) {
	const $i = next($h);
	if ($i[0] !== 0) {
		break;
	}
	const n = $i[1];
	squares.push(Math.pow(n, 2));
}
console.log(squares.length);
