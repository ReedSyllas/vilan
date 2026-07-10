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
function bump(slot) {
	slot[0][slot[1]] = slot[0][slot[1]] + 100;
}
let xs = [  ];
xs.push(10);
xs.push(20);
console.log(__at(xs, 0) + __at(xs, 1));
__at_put(xs, 1, 99);
console.log(__at(xs, 1));
const i = 0;
bump(__at_view(xs, i + 0));
console.log(__at(xs, 0));
let ps = [  ];
ps.push([ 1, 2 ]);
let copy = __clone(__at(ps, 0));
copy[0] = 7;
console.log(__at(ps, 0)[0]);
const view = __at(ps, 0);
view[1] = 50;
console.log(__at(ps, 0)[1]);
console.log(__at(xs, xs.length - 1));
