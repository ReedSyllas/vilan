function pick(a2, b2) {
	return a2 + b2;
}
function pick2(a3, b3) {
	return [ a3[0] + b3[0] ];
}
function $a(a, b) {
	return pick(a, b);
}
function $b(a, b) {
	return pick2(a, b);
}
console.log($a(2, 3));
console.log($b([ 10 ], [ 20 ]));
