function diff(self, other) {
	let $a = null;
	if (self > other) {
		$a = self - other;
	} else {
		$a = other - self;
	}
	return $a;
}
const n = -(5);
console.log(Math.abs(n));
console.log(diff(n, 3));
const b = 2;
console.log(Math.pow(b, 10));
console.log(Math.min(b, 7));
console.log(Math.max(b, 7));
const x = 16;
console.log(Math.sqrt(x));
const y = 3.7;
console.log(Math.floor(y));
console.log(Math.ceil(y));
console.log(Math.round(y));
console.log(Math.min(y, 2));
console.log(Math.max(y, 10));
