function sum(self) {
	let total = default2();
	let seeded = false;
	for (const item of self) {
		if (seeded) {
			total = total + item;
		} else {
			total = item;
			seeded = true;
		}
	}
	return total;
}
function product(self2) {
	let total2 = default2();
	let seeded2 = false;
	for (const item2 of self2) {
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
function default3() {
	return 0;
}
function $a(self) {
	let total = default3();
	let seeded = false;
	for (const item of self) {
		if (seeded) {
			total = total + item;
		} else {
			total = item;
			seeded = true;
		}
	}
	return total;
}
let numbers = [  ];
numbers.push(2);
numbers.push(3);
numbers.push(4);
console.log(sum(numbers));
console.log(product(numbers));
const empty = [  ];
console.log($a(empty));
for (const n of numbers) {
	console.log(n);
}
