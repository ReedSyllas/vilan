function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function $a(self) {
	return self.size === 0;
}
let numbers = new Set();
numbers.add(1);
numbers.add(2);
numbers.add(2);
numbers.add(3);
console.log(numbers.size);
console.log(numbers.has(2));
console.log(numbers.has(9));
numbers.delete(2);
console.log(numbers.has(2));
console.log(numbers.size);
console.log($a(numbers));
let total = 0;
for (const value of numbers) {
	total = total + value;
}
console.log(total);
let copy = __clone(numbers);
copy.add(100);
console.log(numbers.has(100));
console.log(copy.has(100));
let words = new Set();
words.add("hi");
words.add("hi");
words.add("bye");
console.log(words.size);
console.log(words.has("hi"));
let empty = new Set();
console.log($a(empty));
