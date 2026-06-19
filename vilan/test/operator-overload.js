function add(self, b2) {
	return [ self[0] + b2[0], self[1] + b2[1] ];
}
function mul(self2, b3) {
	return [ self2[0] * b3[0], self2[1] * b3[1] ];
}
const a = [ 1, 2 ];
const b = [ 3, 4 ];
const sum = add(a, b);
console.log(sum[0]);
console.log(sum[1]);
const product = mul(a, b);
console.log(product[0]);
console.log(product[1]);
