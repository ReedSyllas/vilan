function increment(self) {
	self[0] = self[0] + 1;
}
function bump(c2) {
	c2[0] = c2[0] + 10;
}
let c = [ 10 ];
increment(c);
bump(c);
console.log(c[0]);
