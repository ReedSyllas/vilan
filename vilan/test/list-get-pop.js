function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	return value;
}
function __list_get(list, index) {
	return index >= 0 && index < list.length ? [ 0, __clone(list[index]) ] : [ 1 ];
}
function __list_pop(list) {
	return list.length === 0 ? [ 1 ] : [ 0, list.pop() ];
}
function k/*first*/(l) {
	return __list_get(l, 0);
}
function m/*last*/(n) {
	return __list_get(n, n.length - 1);
}
function b(c, d) {
	const e = c;
	let f = null;
	if (e[0] === 0) {
		const g/*x*/ = e[1];
		f = g/*x*/;
	} else {
		f = d;
	}
	return f;
}
function h(i) {
	const j = i;
	return j[0] === 1;
}
let a/*xs*/ = [  ];
a/*xs*/.push(10);
a/*xs*/.push(20);
a/*xs*/.push(30);
console.log(b(__list_get(a/*xs*/, 0), 0));
console.log(b(__list_get(a/*xs*/, 2), 0));
console.log(b(__list_get(a/*xs*/, 5), 0));
console.log(h(__list_get(a/*xs*/, 9)));
console.log(b(k/*first*/(a/*xs*/), 0));
console.log(b(m/*last*/(a/*xs*/), 0));
console.log(b(__list_pop(a/*xs*/), 0));
console.log(a/*xs*/.length);
console.log(b(m/*last*/(a/*xs*/), 0));
let o/*single*/ = [  ];
o/*single*/.push(7);
console.log(b(__list_pop(o/*single*/), 0));
console.log(h(__list_pop(o/*single*/)));
