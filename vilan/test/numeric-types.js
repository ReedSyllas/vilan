function fold_unsigned(value, modulus) {
	const truncated = Math.trunc(value);
	const wrapped = truncated - Math.trunc(truncated / modulus) * modulus;
	let $b = null;
	if (wrapped < 0) {
		$b = wrapped + modulus;
	} else {
		$b = wrapped;
	}
	return $b;
}
function fold_signed(value, modulus, half) {
	const wrapped = fold_unsigned(value, modulus);
	let $c = null;
	if (wrapped >= half) {
		$c = wrapped - modulus;
	} else {
		$c = wrapped;
	}
	return $c;
}
function as_i8(self) {
	const widened = Number(self);
	return Number(fold_signed(widened, 256, 128));
}
function as_u8(self) {
	const widened = Number(self);
	return Number(fold_unsigned(widened, 256));
}
function as_u16(self) {
	const widened = Number(self);
	return Number(fold_unsigned(widened, 65536));
}
function as_i32(self) {
	const widened = Number(self);
	return Number(fold_signed(widened, 4294967296, 2147483648));
}
function as_i64(self) {
	const widened = Number(self);
	return Number(Math.trunc(widened));
}
function as_i322(self) {
	const widened = self;
	return Number(fold_signed(widened, 4294967296, 2147483648));
}
function div(self, b) {
	return Math.trunc(self / b);
}
function to_json(self) {
	return "{\"kind\":" + JSON.stringify(self[0]) + "," + "\"sequence\":" + JSON.stringify(self[1]) + "," + "\"stamp\":" + JSON.stringify(self[2]) + "}";
}
function $a(value, divisor) {
	return div(value, divisor);
}
const byte = 0xFF;
const short = 60000;
const wide = 9007199254740992;
const ratio = 2.5;
console.log(byte);
console.log(short);
console.log(wide);
console.log(ratio);
console.log(Math.trunc(7 / 2));
console.log(Math.trunc(-(7) / 2));
console.log(Math.trunc(7 / 2));
console.log(Math.trunc(100 / 3));
console.log(7.0 / 2.0);
console.log(7n / 2n);
let counter = 9;
counter = Math.trunc(counter / 2);
console.log(counter);
console.log($a(100, 8));
console.log(as_u8(300));
console.log(as_u8(-(1)));
console.log(as_i8(130));
console.log(as_i322(3.9));
console.log(as_i322(-(3.9)));
console.log(as_u16(70000));
console.log(Number(byte) + 0.25);
console.log(as_i32(wide));
console.log(as_i64(2.5));
const doubled = 100 + 100;
console.log(doubled);
console.log(100 * 3);
console.log(5 < 6);
console.log(JSON.stringify(200));
const packet = [ 7, 300, 5 ];
console.log(to_json(packet));
