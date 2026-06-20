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
function $b(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $a(value) {
	return [ $b(value) ];
}
function $e(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $f(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		const notify = subscriber[1];
		notify();
	}
}
function $d(self, transform) {
	const derived = $e(transform(self[0].v));
	const upstream = __clone(self[0]);
	self[1].v.push([ fresh_id(), () => {
		$f(derived, transform(upstream.v));
		return;
	} ]);
	return derived;
}
function $c(self, transform) {
	return $d(self[0], transform);
}
function $g(self, observer) {
	const upstream = __clone(self[0]);
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer(upstream.v);
		return;
	} ]);
	observer(self[0].v);
	return [ self[1], id ];
}
function $i(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		const notify = subscriber[1];
		notify();
	}
}
function $h(self, value) {
	$i(self[0], value);
}
function $k(self) {
	return self[0].v;
}
function $j(self, transform) {
	$i(self[0], transform($k(self[0])));
}
function $l(self) {
	return self[0].v;
}
function $n(self, observer) {
	const upstream = __clone(self[0]);
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer(upstream.v);
		return;
	} ]);
	observer(self[0].v);
	return [ self[1], id ];
}
function $m(self, observer) {
	return $n(self[0], observer);
}
const next_subscriber_id = __shared_new(0);
const count = $a(0);
const doubled = $c(count, (n) => {
	return n * 2;
});
$g(doubled, (n) => {
	return console.log(n);
});
$h(count, 1);
$j(count, (n) => {
	return n + 4;
});
console.log($l(doubled));
$m(count, (n) => {
	return console.log(n);
});
$h(count, 20);
