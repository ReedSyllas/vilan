function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function __map_get(map, key) {
	return map.has(key) ? [ 0, __clone(map.get(key)) ] : [ 1 ];
}
function __map_keys(map) {
	return [ ...map.keys() ].map(__clone);
}
function __map_values(map) {
	return [ ...map.values() ].map(__clone);
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
function $d(self2) {
	const $e = self2;
	return $e[0] === 0;
}
function $f(self3) {
	return self3.size === 0;
}
function $g(self, fallback) {
	const $h = self;
	let $i = null;
	if ($h[0] === 0) {
		const x = $h[1];
		$i = x;
	} else {
		$i = fallback;
	}
	return $i;
}
let scores = new Map();
scores.set("alice", 1);
scores.set("bob", 2);
scores.set("carol", 3);
console.log(scores.size);
console.log(scores.has("bob"));
console.log(scores.has("dave"));
console.log($a(__map_get(scores, "bob"), 0));
console.log($a(__map_get(scores, "dave"), -(1)));
console.log($d(__map_get(scores, "alice")));
scores.set("bob", 22);
console.log($a(__map_get(scores, "bob"), 0));
console.log(scores.size);
scores.delete("bob");
console.log(scores.has("bob"));
console.log(scores.size);
console.log($f(scores));
let copy = __clone(scores);
copy.set("dave", 4);
console.log(scores.has("dave"));
console.log(copy.has("dave"));
let names = new Map();
names.set(1, "one");
names.set(2, "two");
console.log($g(__map_get(names, 1), "?"));
console.log($g(__map_get(names, 9), "?"));
let letters = new Map();
letters.set("a", 10);
letters.set("b", 20);
letters.set("c", 30);
let key_count = 0;
for (const key of __map_keys(letters)) {
	key_count = key_count + 1;
}
console.log(key_count);
let sum = 0;
for (const value of __map_values(letters)) {
	sum = sum + value;
}
console.log(sum);
console.log(__map_keys(letters).length);
let empty = new Map();
console.log($f(empty));
