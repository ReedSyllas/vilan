function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function __list_get(list, index) {
	return index >= 0 && index < list.length ? [ 0, __clone(list[index]) ] : [ 1 ];
}
function __list_pop(list) {
	return list.length === 0 ? [ 1 ] : [ 0, list.pop() ];
}
function first(self) {
	return __list_get(self, 0);
}
function last(self) {
	return __list_get(self, self.length - 1);
}
function $a(self, fallback) {
	const $b = self;
	let $c = null;
	if ($b[0] === 0) {
		const x = $b[1];
		$c = x;
	} else {
		$c = fallback;
	}
	return $c;
}
function $d(self) {
	const $e = self;
	return $e[0] === 1;
}
let xs = [  ];
xs.push(10);
xs.push(20);
xs.push(30);
console.log($a(__list_get(xs, 0), 0));
console.log($a(__list_get(xs, 2), 0));
console.log($a(__list_get(xs, 5), 0));
console.log($d(__list_get(xs, 9)));
console.log($a(first(xs), 0));
console.log($a(last(xs), 0));
console.log($a(__list_pop(xs), 0));
console.log(xs.length);
console.log($a(last(xs), 0));
let single = [  ];
single.push(7);
console.log($a(__list_pop(single), 0));
console.log($d(__list_pop(single)));
