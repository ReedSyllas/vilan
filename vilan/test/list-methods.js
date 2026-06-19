function is_empty(self) {
	return self.length === 0;
}
function map(self, fn) {
	let result = [  ];
	for (const item of self) {
		result.push(fn(item));
	}
	return result;
}
function filter(self, predicate) {
	let result = [  ];
	for (const item of self) {
		if (predicate(item)) {
			result.push(item);
		}
	}
	return result;
}
function fold(self, init, fn) {
	let accumulator = init;
	for (const item of self) {
		accumulator = fn(accumulator, item);
	}
	return accumulator;
}
function for_each(self, fn) {
	for (const item of self) {
		fn(item);
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
console.log(filter(xs, (n) => {
	return n > 2;
}).length);
console.log(filter(xs, (n) => {
	return n > 5;
}).length);
for_each(xs, (n) => {
	return console.log(n);
});
