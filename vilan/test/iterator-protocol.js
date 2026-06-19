function sum(self2) {
	let total = default2();
	let seeded = false;
	for (const item of self2) {
		if (seeded) {
			total = total + item;
		} else {
			total = item;
			seeded = true;
		}
	}
	return total;
}
function product(self3) {
	let total2 = default2();
	let seeded2 = false;
	for (const item2 of self3) {
		if (seeded2) {
			total2 = total2 * item2;
		} else {
			total2 = item2;
			seeded2 = true;
		}
	}
	return total2;
}
function default2() {

}
function next(self) {
	produced = produced + 1;
	let $a = null;
	if (produced <= self[0]) {
		$a = [ 0, produced ];
	} else {
		$a = [ 1 ];
	}
	return $a;
}
let produced = 0;
const naturals = [ 3 ];
const $b = naturals;
while (true) {
	const $c = next($b);
	if ($c[0] !== 0) {
		break;
	}
	const n = $c[1];
	console.log(n);
}
let numbers = [  ];
numbers.push(2);
numbers.push(3);
numbers.push(4);
console.log(sum(numbers));
console.log(product(numbers));
