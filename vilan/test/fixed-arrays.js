function __at(list, index) {
	if (index >= 0 && index < list.length) return list[index];
	throw "index out of bounds: the length is " + list.length + " but the index is " + index;
}
function __at_put(list, index, value) {
	if (index >= 0 && index < list.length) return list[index] = value;
	throw "index out of bounds: the length is " + list.length + " but the index is " + index;
}
function __at_view(list, index) {
	if (index >= 0 && index < list.length) return [ list, index ];
	throw "index out of bounds: the length is " + list.length + " but the index is " + index;
}
function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function __repeat(value, n) {
	return typeof value === "object" && value !== null
		? Array.from({ length: n }, () => __clone(value))
		: new Array(n).fill(value);
}
function total(values) {
	let sum = 0;
	for (const value of values) {
		sum = sum + value;
	}
	return sum;
}
function make() {
	return __repeat(5, 2);
}
function bump(view2) {
	view2[0][view2[1]] = view2[0][view2[1]] + 100;
}
const zeros = __repeat(0, 4);
console.log(__at(zeros, 0));
let buf = [ 1, 2, 3 ];
__at_put(buf, 1, 20);
console.log(__at(buf, 1));
console.log(total(buf));
const view = __at_view(buf, 2);
bump(view);
console.log(__at(buf, 2));
const copy = __clone(buf);
__at_put(buf, 0, 99);
console.log(__at(copy, 0));
let cells = __repeat([ 7 ], 3);
__at(cells, 0)[0] = 42;
console.log(__at(cells, 0)[0]);
console.log(__at(cells, 1)[0]);
const two = make();
console.log(__at(two, 0) + __at(two, 1));
const grid = [ [ 1, 2 ], [ 3, 4 ] ];
console.log(__at(__at(grid, 1), 0));
console.log(3);
console.log(__at(grid, 0).length);
console.log(make().length);
