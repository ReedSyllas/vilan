function combine(self, b) {
	return [ self[0] + b[0] ];
}
function $a(self) {
	return combine(combine(self, self), self);
}
const c = [ 5 ];
console.log($a(c)[0]);
