function add_ten(x) {
	x[0][x[1]] = x[0][x[1]] + 10;
}
function same(x) {
	return x;
}
function slot(self) {
	return [ 0, [ self, 0 ] ];
}
let a = [ 10 ];
const b = [ a, 0 ];
const c = b;
b[0][b[1]] = 20;
console.log("" + a[0] + " " + b[0][b[1]] + " " + c[0][c[1]]);
add_ten([ a, 0 ]);
console.log("" + a[0] + " " + b[0][b[1]]);
add_ten(b);
console.log("" + a[0] + " " + b[0][b[1]]);
const $a = same(c);
const $b = same(c);
$b[0][$b[1]] = Math.trunc($a[0][$a[1]] / 10);
console.log("" + a[0] + " " + b[0][b[1]]);
let cell = [ 100 ];
const $c = slot(cell);
let $d = null;
if ($c[0] === 0) {
	const s = $c[1];
	s[0][s[1]] = s[0][s[1]] + 5;
	$d = undefined;
} else {
	$d = undefined;
}
$d;
console.log(cell[0]);
