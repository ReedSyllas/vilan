function is_empty(self) {
	return self.length === 0;
}
function map(self2, fn) {
	let result = [  ];
	for (const item of self2) {
		result.push(fn(item));
	}
	return result;
}
function filter(self4, predicate) {
	let result2 = [  ];
	for (const item3 of self4) {
		if (predicate(item3)) {
			result2.push(item3);
		}
	}
	return result2;
}
function fold(self3, init, fn2) {
	let accumulator = init;
	for (const item2 of self3) {
		accumulator = fn2(accumulator, item2);
	}
	return accumulator;
}
function for_each(self5, fn3) {
	for (const item4 of self5) {
		fn3(item4);
	}
}
let xs = [  ];
xs.push(1);
xs.push(2);
xs.push(3);
xs.push(4);
console.log(xs.length);
console.log(is_empty(xs));
console.log(fold(map(xs, (n) => {
	return n * 10;
}), 0, (a, b) => {
	return a + b;
}));
console.log(filter(xs, (n2) => {
	return n2 > 2;
}).length);
console.log(filter(xs, (n3) => {
	return n3 > 5;
}).length);
for_each(xs, (n4) => {
	return console.log(n4);
});
