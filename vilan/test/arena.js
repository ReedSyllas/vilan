function __list_pop(list) {
	return list.length === 0 ? [ 1 ] : [ 0, list.pop() ];
}
function sum_from(arena, handle) {
	const $A = $x(arena, handle);
	let $B = null;
	if ($A[0] === 0) {
		const node = $A[1];
		let total = node[0];
		for (const edge of node[1]) {
			total = total + sum_from(arena, edge);
		}
		$B = total;
	} else {
		$B = 0;
	}
	return $B;
}
function $a() {
	return [ [  ], [  ] ];
}
function $b(self, value) {
	const $c = __list_pop(self[1]);
	let $d = null;
	if ($c[0] === 0) {
		const index = $c[1];
		self[0][index][1] = [ 0, value ];
		$d = [ index, self[0][index][0] ];
	} else {
		const index2 = self[0].length;
		self[0].push([ 0, [ 0, value ] ]);
		$d = [ index2, 0 ];
	}
	return $d;
}
function $e(self) {
	return self[0].length - self[1].length;
}
function $h(self) {
	const $i = self;
	return $i[0] === 0;
}
function $g(self, handle) {
	return handle[0] < self[0].length && self[0][handle[0]][0] === handle[1] && $h(self[0][handle[0]][1]);
}
function $f(self, handle) {
	let $j = null;
	if ($g(self, handle)) {
		$j = self[0][handle[0]][1];
	} else {
		$j = [ 1 ];
	}
	return $j;
}
function $k(self, fallback) {
	const $l = self;
	let $m = null;
	if ($l[0] === 0) {
		const x = $l[1];
		$m = x;
	} else {
		$m = fallback;
	}
	return $m;
}
function $n(self, handle, value) {
	let $o = null;
	if ($g(self, handle)) {
		self[0][handle[0]][1] = [ 0, value ];
		$o = true;
	} else {
		$o = false;
	}
	return $o;
}
function $p(self, handle) {
	let $q = null;
	if ($g(self, handle)) {
		const removed = self[0][handle[0]][1];
		self[0][handle[0]][0] = self[0][handle[0]][0] + 1;
		self[0][handle[0]][1] = [ 1 ];
		self[1].push(handle[0]);
		$q = removed;
	} else {
		$q = [ 1 ];
	}
	return $q;
}
function $r(self) {
	const $s = self;
	return $s[0] === 0;
}
function $t() {
	return [ [  ], [  ] ];
}
function $u(self, value) {
	const $v = __list_pop(self[1]);
	let $w = null;
	if ($v[0] === 0) {
		const index = $v[1];
		self[0][index][1] = [ 0, value ];
		$w = [ index, self[0][index][0] ];
	} else {
		const index2 = self[0].length;
		self[0].push([ 0, [ 0, value ] ]);
		$w = [ index2, 0 ];
	}
	return $w;
}
function $y(self, handle) {
	return handle[0] < self[0].length && self[0][handle[0]][0] === handle[1] && $h(self[0][handle[0]][1]);
}
function $x(self, handle) {
	let $z = null;
	if ($y(self, handle)) {
		$z = self[0][handle[0]][1];
	} else {
		$z = [ 1 ];
	}
	return $z;
}
let numbers = $a();
const a = $b(numbers, 10);
const b = $b(numbers, 20);
console.log($e(numbers));
console.log($k($f(numbers, a), -(1)));
$n(numbers, b, 99);
console.log($k($f(numbers, b), -(1)));
console.log($k($p(numbers, b), -(1)));
console.log($r($f(numbers, b)));
const c = $b(numbers, 30);
console.log($k($f(numbers, c), -(1)));
console.log($r($f(numbers, b)));
console.log($k($f(numbers, a), -(1)));
let graph = $t();
const leaf1 = $u(graph, [ 2, [  ] ]);
const leaf2 = $u(graph, [ 3, [  ] ]);
let root_edges = [  ];
root_edges.push(leaf1);
root_edges.push(leaf2);
const root = $u(graph, [ 1, root_edges ]);
console.log(sum_from(graph, root));
