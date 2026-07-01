function __shared_new(value) {
	return { v: value };
}
function fresh_id() {
	const id = next_subscriber_id.v;
	next_subscriber_id.v = id + 1;
	return id;
}
function dispose(self) {
	let kept = [  ];
	for (const subscriber of self[0].v) {
		if (subscriber[0] !== self[1]) {
			kept.push(subscriber);
		}
	}
	self[0].v = kept;
}
function new2() {
	return [ __shared_new([  ]) ];
}
function $a(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $c(self) {
	return self[0].v;
}
function $d(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		subscriber[1]();
	}
}
function $b(self, transform) {
	const derived = $a(transform($c(self)));
	self[1].v.push([ fresh_id(), () => {
		$d(derived, transform($c(self)));
		return;
	} ]);
	return derived;
}
function $e(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($c(self));
		return;
	} ]);
	observer($c(self));
	return [ self[1], id ];
}
function $f(self, item) {
	self[0].v.push(() => {
		dispose(item);
		return;
	});
	return item;
}
function $g(self, transform) {
	$d(self, transform($c(self)));
}
const next_subscriber_id = __shared_new(0);
const owner = new2();
const count = $a(0);
const doubled = $b(count, (n) => {
	return n * 2;
});
$f(owner, $e(doubled, (n) => {
	return console.log(n);
}));
$d(count, 1);
$g(count, (n) => {
	return n + 4;
});
console.log($c(doubled));
$f(owner, $e(count, (n) => {
	return console.log(n);
}));
$d(count, 20);
