function get_mut(self) {
	return [ 0, [ self, 0 ] ];
}
function get(self2) {
	return [ 0, [ self2, 0 ] ];
}
function inner_mut(self3) {
	return [ 0, self3[0] ];
}
function item_mut(self4, index) {
	let $g = null;
	if (index < self4[1].length) {
		$g = [ 0, [ self4[1], index ] ];
	} else {
		$g = [ 1 ];
	}
	return $g;
}
let slot = [ 1 ];
const $a = get_mut(slot);
let $b = null;
if ($a[0] === 0) {
	const v = $a[1];
	v[0][v[1]] = 42;
	$b = undefined;
} else {
	$b = undefined;
}
$b;
console.log(slot[0]);
const $c = get(slot);
let $d = null;
if ($c[0] === 0) {
	const v2 = $c[1];
	console.log(v2[0][v2[1]]);
	$d = undefined;
} else {
	$d = undefined;
}
$d;
let outer = [ [ 1 ], [ 10, 20, 30 ] ];
const $e = inner_mut(outer);
let $f = null;
if ($e[0] === 0) {
	const v3 = $e[1];
	v3[0] = 77;
	$f = undefined;
} else {
	$f = undefined;
}
$f;
console.log(outer[0][0]);
const $h = item_mut(outer, 1);
let $i = null;
if ($h[0] === 0) {
	const v4 = $h[1];
	v4[0][v4[1]] = 99;
	$i = undefined;
} else {
	$i = undefined;
}
$i;
console.log(outer[1][1]);
const $j = item_mut(outer, 9);
let $k = null;
if ($j[0] === 0) {
	const v5 = $j[1];
	v5[0][v5[1]] = 0;
	$k = undefined;
} else {
	console.log(0);
	$k = undefined;
}
process.exit($k);
