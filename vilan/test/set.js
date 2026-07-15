function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function __hash(value) {
	return (typeof value === "object" && value !== null) ? JSON.stringify(value) : value;
}
function __set_iter(set) {
	return [ ...set[0].values() ];
}
function hash(self) {
	return __hash(self);
}
function hash2(self) {
	return __hash(self);
}
function $a() {
	const table = new Map();
	return [ table ];
}
function $b(self, value2) {
	self[0].set(hash2(value2), value2);
}
function $c(self) {
	return self[0].size;
}
function $d(self, value2) {
	return self[0].has(hash2(value2));
}
function $e(self, value2) {
	self[0].delete(hash2(value2));
}
function $f(self) {
	return $c(self) === 0;
}
function $g() {
	const table = new Map();
	return [ table ];
}
function $h(self, value2) {
	self[0].set(hash(value2), value2);
}
function $i(self) {
	return self[0].size;
}
function $j(self, value2) {
	return self[0].has(hash(value2));
}
let numbers = $a();
$b(numbers, 1);
$b(numbers, 2);
$b(numbers, 2);
$b(numbers, 3);
console.log($c(numbers));
console.log($d(numbers, 2));
console.log($d(numbers, 9));
$e(numbers, 2);
console.log($d(numbers, 2));
console.log($c(numbers));
console.log($f(numbers));
let total = 0;
for (const value of __set_iter(numbers)) {
	total = total + value;
}
console.log(total);
let copy = __clone(numbers);
$b(copy, 100);
console.log($d(numbers, 100));
console.log($d(copy, 100));
let words = $g();
$h(words, "hi");
$h(words, "hi");
$h(words, "bye");
console.log($i(words));
console.log($j(words, "hi"));
let empty = $a();
console.log($f(empty));
