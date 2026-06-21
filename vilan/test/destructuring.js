function to_string(self) {
	return "" + self;
}
function swap(_) {
	const $c = _;
	const a2 = $c[0];
	const b2 = $c[1];
	return [ b2, a2 ];
}
const $a = [ 10, "hi" ];
const x = $a[0];
const y = $a[1];
const $b = [ 1, ...[ 2, "z" ] ];
const n = $b[0];
const m = $b[1];
const label = $b[2];
console.log("" + to_string(x) + " " + y);
console.log("" + to_string(n) + " " + to_string(m) + " " + label);
const $d = swap([ 5, "z" ]);
const p = $d[0];
const q = $d[1];
console.log("" + p + " " + to_string(q));
const scale = (_) => {
	const $e = _;
	const value = $e[0];
	const factor = $e[1];
	return value * factor;
};
console.log(to_string(scale([ 6, 7 ])));
const $f = [ 3, 4 ];
let $g = null;
const a = $f[0];
const b = $f[1];
$g = console.log(to_string(a + b));
$g;
const opt = [ 0, [ 1, "ok" ] ];
const $h = opt;
let $i = null;
if ($h[0] === 0) {
	const first = $h[1][0];
	const second = $h[1][1];
	$i = console.log("" + to_string(first) + " " + second);
} else {
	$i = console.log("none");
}
process.exit($i);
