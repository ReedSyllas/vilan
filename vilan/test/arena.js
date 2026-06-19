function __list_pop(list) {
	return list.length === 0 ? [ 1 ] : [ 0, list.pop() ];
}
function new2() {
	return [ [  ], [  ] ];
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
function $a(self, value) {
	const $b = __list_pop(self[1]);
	let $c = null;
	if ($b[0] === 0) {
		const index = $b[1];
		self[0][index][1] = [ 0, value ];
		$c = [ index, self[0][index][0] ];
	} else {
		const index2 = self[0].length;
		self[0].push([ 0, [ 0, value ] ]);
		$c = [ index2, 0 ];
	}
	return $c;
}
function $d(self) {
	return self[0].length - self[1].length;
}
function $g(self) {
	const $h = self;
	return $h[0] === 0;
}
function $f(self, handle) {
	return handle[0] < self[0].length && self[0][handle[0]][0] === handle[1] && $g(self[0][handle[0]][1]);
}
function $e(self, handle) {
	let $i = null;
	if ($f(self, handle)) {
		$i = self[0][handle[0]][1];
	} else {
		$i = [ 1 ];
	}
	return $i;
}
function $j(self, fallback) {
	const $k = self;
	let $l = null;
	if ($k[0] === 0) {
		const x = $k[1];
		$l = x;
	} else {
		$l = fallback;
	}
	return $l;
}
function $m(self, handle, value) {
	let $n = null;
	if ($f(self, handle)) {
		self[0][handle[0]][1] = [ 0, value ];
		$n = true;
	} else {
		$n = false;
	}
	return $n;
}
function $o(self, handle) {
	let $p = null;
	if ($f(self, handle)) {
		const removed = self[0][handle[0]][1];
		self[0][handle[0]][0] = self[0][handle[0]][0] + 1;
		self[0][handle[0]][1] = [ 1 ];
		self[1].push(handle[0]);
		$p = removed;
	} else {
		$p = [ 1 ];
	}
	return $p;
}
function $q(self) {
	const $r = self;
	return $r[0] === 0;
}
function $s(self, value) {
	const $t = __list_pop(self[1]);
	let $u = null;
	if ($t[0] === 0) {
		const index = $t[1];
		self[0][index][1] = [ 0, value ];
		$u = [ index, self[0][index][0] ];
	} else {
		const index2 = self[0].length;
		self[0].push([ 0, [ 0, value ] ]);
		$u = [ index2, 0 ];
	}
	return $u;
}
function $w(self, handle) {
	return handle[0] < self[0].length && self[0][handle[0]][0] === handle[1] && $g(self[0][handle[0]][1]);
}
function $v(self, handle) {
	let $x = null;
	if ($w(self, handle)) {
		$x = self[0][handle[0]][1];
	} else {
		$x = [ 1 ];
	}
	return $x;
}
let numbers = new2();
const a = $a(numbers, 10);
const b = $a(numbers, 20);
console.log($d(numbers));
console.log($j($e(numbers, a), -(1)));
$m(numbers, b, 99);
console.log($j($e(numbers, b), -(1)));
console.log($j($o(numbers, b), -(1)));
console.log($q($e(numbers, b)));
const c = $a(numbers, 30);
console.log($j($e(numbers, c), -(1)));
console.log($q($e(numbers, b)));
console.log($j($e(numbers, a), -(1)));
let graph = new2();
const leaf1 = $s(graph, [ 2, [  ] ]);
const leaf2 = $s(graph, [ 3, [  ] ]);
let root_edges = [  ];
root_edges.push(leaf1);
root_edges.push(leaf2);
const root = $s(graph, [ 1, root_edges ]);
console.log(sum_from(graph, root));
