function __at(list, index) {
	if (index >= 0 && index < list.length) return list[index];
	throw "index out of bounds: the length is " + list.length + " but the index is " + index;
}
function __list_pop(list) {
	return list.length === 0 ? [ 1 ] : [ 0, list.pop() ];
}
function sum_from(arena, handle) {
	const $y = $v(arena, handle);
	let $z = null;
	if ($y[0] === 0) {
		const node = $y[1];
		let total = node[0];
		for (const edge of node[1]) {
			total = total + sum_from(arena, edge);
		}
		$z = total;
	} else {
		$z = 0;
	}
	return $z;
}
function $a() {
	return [ [  ], [  ] ];
}
function $b(self, value) {
	const $c = __list_pop(self[1]);
	let $d = null;
	if ($c[0] === 0) {
		const index = $c[1];
		__at(self[0], index)[1] = value;
		$d = [ index, __at(self[0], index)[0] ];
	} else {
		const index2 = self[0].length;
		self[0].push([ 0, value ]);
		$d = [ index2, 0 ];
	}
	return $d;
}
function $e(self) {
	return self[0].length - self[1].length;
}
function $g(self, handle) {
	return handle[0] < self[0].length && __at(self[0], handle[0])[0] === handle[1];
}
function $f(self, handle) {
	let $h = null;
	if ($g(self, handle)) {
		$h = [ 0, __at(self[0], handle[0])[1] ];
	} else {
		$h = [ 1 ];
	}
	return $h;
}
function $i(self, fallback) {
	const $j = self;
	let $k = null;
	if ($j[0] === 0) {
		const x = $j[1];
		$k = x;
	} else {
		$k = fallback;
	}
	return $k;
}
function $l(self, handle, value) {
	let $m = null;
	if ($g(self, handle)) {
		__at(self[0], handle[0])[1] = value;
		$m = true;
	} else {
		$m = false;
	}
	return $m;
}
function $n(self, handle) {
	let $o = null;
	if ($g(self, handle)) {
		const removed = __at(self[0], handle[0])[1];
		__at(self[0], handle[0])[0] = __at(self[0], handle[0])[0] + 1;
		self[1].push(handle[0]);
		$o = [ 0, removed ];
	} else {
		$o = [ 1 ];
	}
	return $o;
}
function $p(self) {
	const $q = self;
	return $q[0] === 0;
}
function $r() {
	return [ [  ], [  ] ];
}
function $s(self, value) {
	const $t = __list_pop(self[1]);
	let $u = null;
	if ($t[0] === 0) {
		const index = $t[1];
		__at(self[0], index)[1] = value;
		$u = [ index, __at(self[0], index)[0] ];
	} else {
		const index2 = self[0].length;
		self[0].push([ 0, value ]);
		$u = [ index2, 0 ];
	}
	return $u;
}
function $w(self, handle) {
	return handle[0] < self[0].length && __at(self[0], handle[0])[0] === handle[1];
}
function $v(self, handle) {
	let $x = null;
	if ($w(self, handle)) {
		$x = [ 0, __at(self[0], handle[0])[1] ];
	} else {
		$x = [ 1 ];
	}
	return $x;
}
let numbers = $a();
const a = $b(numbers, 10);
const b = $b(numbers, 20);
console.log($e(numbers));
console.log($i($f(numbers, a), -(1)));
$l(numbers, b, 99);
console.log($i($f(numbers, b), -(1)));
console.log($i($n(numbers, b), -(1)));
console.log($p($f(numbers, b)));
const c = $b(numbers, 30);
console.log($i($f(numbers, c), -(1)));
console.log($p($f(numbers, b)));
console.log($i($f(numbers, a), -(1)));
let graph = $r();
const leaf1 = $s(graph, [ 2, [  ] ]);
const leaf2 = $s(graph, [ 3, [  ] ]);
let root_edges = [  ];
root_edges.push(leaf1);
root_edges.push(leaf2);
const root = $s(graph, [ 1, root_edges ]);
console.log(sum_from(graph, root));
