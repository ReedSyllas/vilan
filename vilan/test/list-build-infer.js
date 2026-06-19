function sum2(self2) {
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
function default2() {

}
function sum(self) {
	return self[0] + self[1];
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
console.log(sum2(numbers));
