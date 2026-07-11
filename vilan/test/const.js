function __at(list, index) {
	if (index >= 0 && index < list.length) return list[index];
	throw "index out of bounds: the length is " + list.length + " but the index is " + index;
}
function square(n) {
	return n * n;
}
const STEPS = [ 1, 2, 4 ];
const folded = 7;
console.log(folded);
const narrowed = 16 + square(2);
console.log(narrowed);
const base = 9;
const doubled = 18;
console.log(doubled);
const total = 7;
console.log(total);
console.log(__at(STEPS, 0) + __at(STEPS, 1) + __at(STEPS, 2));
let cache = 100;
cache = cache + 1;
console.log(cache);
