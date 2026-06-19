function __list_pop(list) {
	return list.length === 0 ? [ 1 ] : [ 0, list.pop() ];
}
function new2() {
	return [ [  ], [  ] ];
}
function sum_from(arena, handle5) {
	const $y = $v(arena, handle5);
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
function $d(self2) {
	return self2[0].length - self2[1].length;
}
function $g(self5) {
	const $h = self5;
	return $h[0] === 0;
}
function $f(self4, handle2) {
	return handle2[0] < self4[0].length && self4[0][handle2[0]][0] === handle2[1] && $g(self4[0][handle2[0]][1]);
}
function $e(self3, handle) {
	let $i = null;
	if ($f(self3, handle)) {
		$i = self3[0][handle[0]][1];
	} else {
		$i = [ 1 ];
	}
	return $i;
}
function $j(self6, fallback) {
	const $k = self6;
	let $l = null;
	if ($k[0] === 0) {
		const x = $k[1];
		$l = x;
	} else {
		$l = fallback;
	}
	return $l;
}
function $m(self7, handle3, value2) {
	let $n = null;
	if ($f(self7, handle3)) {
		self7[0][handle3[0]][1] = [ 0, value2 ];
		$n = true;
	} else {
		$n = false;
	}
	return $n;
}
function $o(self8, handle4) {
	let $p = null;
	if ($f(self8, handle4)) {
		const removed = self8[0][handle4[0]][1];
		self8[0][handle4[0]][0] = self8[0][handle4[0]][0] + 1;
		self8[0][handle4[0]][1] = [ 1 ];
		self8[1].push(handle4[0]);
		$p = removed;
	} else {
		$p = [ 1 ];
	}
	return $p;
}
function $q(self5) {
	const $r = self5;
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
function $w(self4, handle2) {
	return handle2[0] < self4[0].length && self4[0][handle2[0]][0] === handle2[1] && $g(self4[0][handle2[0]][1]);
}
function $v(self3, handle) {
	let $x = null;
	if ($w(self3, handle)) {
		$x = self3[0][handle[0]][1];
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
