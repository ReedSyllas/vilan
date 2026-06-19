function increment(self) {
	self[0] = self[0] + 1;
}
function bump(self, by) {
	self[0] = self[0] + by;
}
let c = [ 10 ];
c[0] = 5;
console.log(c[0]);
increment(c);
console.log(c[0]);
bump(c, 100);
console.log(c[0]);
let x = 1;
x = 2;
x = x + 3;
console.log(x);
