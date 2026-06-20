function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function __shared_new(value) {
	return { v: value };
}
function fresh_id() {
	const id = next_subscriber_id.v;
	next_subscriber_id.v = id + 1;
	return id;
}
function new2(value) {
	return [ $a(value) ];
}
function $a(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $d(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		const notify = subscriber[1];
		notify();
	}
}
function $c(self, transform) {
	const derived = $a(transform(self[0].v));
	const upstream = __clone(self[0]);
	self[1].v.push([ fresh_id(), () => {
		$d(derived, transform(upstream.v));
		return;
	} ]);
	return derived;
}
function $b(self, transform) {
	return $c(self[0], transform);
}
function $e(self, observer) {
	const upstream = __clone(self[0]);
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer(upstream.v);
		return;
	} ]);
	observer(self[0].v);
	return [ self[1], id ];
}
function $f(self, value) {
	$d(self[0], value);
}
function $h(self) {
	return self[0].v;
}
function $g(self, transform) {
	$d(self[0], transform($h(self[0])));
}
function $i(self) {
	return self[0].v;
}
function $k(self, observer) {
	const upstream = __clone(self[0]);
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer(upstream.v);
		return;
	} ]);
	observer(self[0].v);
	return [ self[1], id ];
}
function $j(self, observer) {
	return $k(self[0], observer);
}
const next_subscriber_id = __shared_new(0);
const count = new2(0);
const doubled = $b(count, (n) => {
	return n * 2;
});
$e(doubled, (n) => {
	return console.log(n);
});
$f(count, 1);
$g(count, (n) => {
	return n + 4;
});
console.log($i(doubled));
$j(count, (n) => {
	return console.log(n);
});
$f(count, 20);
