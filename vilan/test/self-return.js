function combine(self2, b) {
	return [ self2[0] + b[0] ];
}
function $a(self) {
	return combine(combine(self, self), self);
}
const c = [ 5 ];
console.log($a(c)[0]);
