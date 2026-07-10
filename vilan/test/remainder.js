function rem(self, b) {
	return [ self[0] % b[0] ];
}
console.log(7 % 3);
console.log((0 - 7) % 3);
console.log(7 % (0 - 3));
console.log(7.5 % 2);
console.log(9000000000000000 % 7);
console.log(4000000000 % 7);
console.log(9007199254740993n % 4n);
console.log(1 + 7 % 3);
console.log(7 % 3 * 2);
let x = 17;
x = x % 5;
console.log(x);
const left = [ 17 ];
const right = [ 5 ];
const paced = rem(left, right);
console.log(paced[0]);
