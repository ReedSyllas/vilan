function default2() {
	return 0;
}
function sum(self) {
	return self[0] + self[1];
}
function $a(self) {
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
let points = [  ];
points.push([ 1, 2 ]);
points.push([ 3, 4 ]);
for (const point of points) {
	console.log(sum(point));
}
let numbers = [  ];
numbers.push(10);
numbers.push(20);
numbers.push(30);
console.log($a(numbers));
