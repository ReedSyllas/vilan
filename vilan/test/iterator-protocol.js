function default2() {
	return 0;
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
function $d(self) {
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
function $e(self) {
	let total = default2();
	let seeded = false;
	for (const item of self) {
		if (seeded) {
			total = total * item;
		} else {
			total = item;
			seeded = true;
		}
	}
	return total;
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
console.log($d(numbers));
console.log($e(numbers));
