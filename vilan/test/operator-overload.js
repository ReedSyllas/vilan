function add(self, b2) {
	return [ self[0] + b2[0], self[1] + b2[1] ];
}
function mul(self, b2) {
	return [ self[0] * b2[0], self[1] * b2[1] ];
}
const a = [ 1, 2 ];
const b = [ 3, 4 ];
const sum = add(a, b);
console.log(sum[0]);
console.log(sum[1]);
const product = mul(a, b);
console.log(product[0]);
console.log(product[1]);
