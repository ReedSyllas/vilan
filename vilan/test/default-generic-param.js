function value_of(self2) {
	return self2[0];
}
function $a(self, b2) {
	return value_of(self) + value_of(b2);
}
const a = [ 4 ];
const b = [ 6 ];
console.log($a(a, b));
