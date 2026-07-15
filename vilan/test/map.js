function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function __hash(value) {
	return (typeof value === "object" && value !== null) ? JSON.stringify(value) : value;
}
function __map_get(map, key) {
	return map.has(key) ? [ 0, __clone(map.get(key)) ] : [ 1 ];
}
function __map_values(map) {
	return [ ...map.values() ].map(__clone);
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
function $b(self, key2, value2) {
	self[0].set(hash(key2), [ key2, value2 ]);
}
function $c(self) {
	return self[0].size;
}
function $d(self, key2) {
	return self[0].has(hash(key2));
}
function $e(self, key2) {
	const $f = __map_get(self[0], hash(key2));
	let $g = null;
	if ($f[0] === 0) {
		const entry = $f[1];
		$g = [ 0, entry[1] ];
	} else {
		$g = [ 1 ];
	}
	return $g;
}
function $h(self, fallback) {
	const $i = self;
	let $j = null;
	if ($i[0] === 0) {
		const x = $i[1];
		$j = x;
	} else {
		$j = fallback;
	}
	return $j;
}
function $k(self) {
	const $l = self;
	return $l[0] === 0;
}
function $m(self, key2) {
	self[0].delete(hash(key2));
}
function $n(self) {
	return $c(self) === 0;
}
function $o() {
	const table = new Map();
	return [ table ];
}
function $p(self, key2, value2) {
	self[0].set(hash2(key2), [ key2, value2 ]);
}
function $q(self, key2) {
	const $r = __map_get(self[0], hash2(key2));
	let $s = null;
	if ($r[0] === 0) {
		const entry = $r[1];
		$s = [ 0, entry[1] ];
	} else {
		$s = [ 1 ];
	}
	return $s;
}
function $t(self, fallback) {
	const $u = self;
	let $v = null;
	if ($u[0] === 0) {
		const x = $u[1];
		$v = x;
	} else {
		$v = fallback;
	}
	return $v;
}
function $w(self) {
	let result = [  ];
	for (const entry of __map_values(self[0])) {
		result.push(entry[0]);
	}
	return result;
}
function $x(self) {
	let result = [  ];
	for (const entry of __map_values(self[0])) {
		result.push(entry[1]);
	}
	return result;
}
let scores = $a();
$b(scores, "alice", 1);
$b(scores, "bob", 2);
$b(scores, "carol", 3);
console.log($c(scores));
console.log($d(scores, "bob"));
console.log($d(scores, "dave"));
console.log($h($e(scores, "bob"), 0));
console.log($h($e(scores, "dave"), -(1)));
console.log($k($e(scores, "alice")));
$b(scores, "bob", 22);
console.log($h($e(scores, "bob"), 0));
console.log($c(scores));
$m(scores, "bob");
console.log($d(scores, "bob"));
console.log($c(scores));
console.log($n(scores));
let copy = __clone(scores);
$b(copy, "dave", 4);
console.log($d(scores, "dave"));
console.log($d(copy, "dave"));
let names = $o();
$p(names, 1, "one");
$p(names, 2, "two");
console.log($t($q(names, 1), "?"));
console.log($t($q(names, 9), "?"));
let letters = $a();
$b(letters, "a", 10);
$b(letters, "b", 20);
$b(letters, "c", 30);
let key_count = 0;
for (const key of $w(letters)) {
	key_count = key_count + 1;
}
console.log(key_count);
let sum = 0;
for (const value of $x(letters)) {
	sum = sum + value;
}
console.log(sum);
console.log($w(letters).length);
let empty = $a();
console.log($n(empty));
