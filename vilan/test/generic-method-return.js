function $a(self) {
	return self[0];
}
function $b(self2) {
	return [ 0, self2[0] ];
}
const b = [ [ 5 ] ];
console.log($a(b)[0]);
const $c = $b(b);
let $d = null;
if ($c[0] === 0) {
	const n = $c[1];
	$d = console.log(n[0]);
} else {
	$d = console.log(-(1));
}
process.exit($d);
