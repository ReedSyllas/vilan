function pick(a, b) {
	return a + b;
}
function pick2(a, b) {
	return [ a[0] + b[0] ];
}
function $a(a, b) {
	return pick(a, b);
}
function $b(a, b) {
	return pick2(a, b);
}
console.log($a(2, 3));
console.log($b([ 10 ], [ 20 ]));
