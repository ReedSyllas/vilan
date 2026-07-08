function __shared_new(value) {
	return { v: value };
}
function fresh_id() {
	const id = next_subscriber_id.v;
	next_subscriber_id.v = id + 1;
	return id;
}
function enqueue(subscribers) {
	for (const subscriber of subscribers) {
		let seen = false;
		for (const queued of scheduler[0].v) {
			if (queued[0] === subscriber[0]) {
				seen = true;
			}
		}
		if (!(seen)) {
			scheduler[0].v.push(subscriber);
		}
	}
}
function dispose(self) {
	let kept = [  ];
	for (const subscriber of self[0].v) {
		if (subscriber[0] !== self[1]) {
			kept.push(subscriber);
		}
	}
	self[0].v = kept;
	let kept_pending = [  ];
	for (const subscriber2 of scheduler[0].v) {
		if (subscriber2[0] !== self[1]) {
			kept_pending.push(subscriber2);
		}
	}
	scheduler[0].v = kept_pending;
}
function new2() {
	return [ __shared_new([  ]) ];
}
function dispose2(self) {
	for (const cleanup of self[0].v) {
		cleanup();
	}
	self[0].v = [  ];
}
function get_owner($e) {
	return $e;
}
function $a(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $g(self) {
	return self[0].v;
}
function $f(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($g(self));
		return;
	} ]);
	observer($g(self));
	return [ self[1], id ];
}
function $h(self, item) {
	self[0].v.push(() => {
		dispose(item);
		return;
	});
	return item;
}
function $c(self, observer, $d) {
	$h(get_owner($d), $f(self, observer));
}
function $i(self, value) {
	self[0].v = value;
	let $j = null;
	if (scheduler[1].v === 0) {
		for (const subscriber of self[1].v) {
			subscriber[1]();
		}
		$j = undefined;
	} else {
		enqueue(self[1].v);
	}
	return $j;
}
function $n(owner2, body) {
	return body(owner2);
}
function $p(body) {
	const scope2 = new2();
	const result = body(scope2);
	return [ result, scope2 ];
}
const owner_scope = null;
const next_subscriber_id = __shared_new(0);
const scheduler = [ __shared_new([  ]), __shared_new(0), __shared_new(false) ];
const count = $a(1);
const owner = new2();
(($b) => {
	$c(count, (value) => {
		return console.log("seen " + value);
	}, $b);
	return;
})(owner);
$i(count, 2);
dispose2(owner);
$i(count, 3);
console.log("done");
const outer = new2();
const inner = new2();
(($k) => {
	(($l) => {
		$c(count, (value) => {
			return console.log("inner " + value);
		}, $l);
		return;
	})(inner);
	$c(count, (value) => {
		return console.log("outer " + value);
	}, $k);
	return;
})(outer);
$i(count, 4);
dispose2(inner);
$i(count, 5);
dispose2(outer);
$i(count, 6);
console.log("end");
const wrapped = new2();
$n(wrapped, ($m) => {
	$c(count, (value) => {
		return console.log("wrapped " + value);
	}, $m);
	return;
});
$i(count, 7);
dispose2(wrapped);
$i(count, 8);
console.log("fin");
const $q = $p(($o) => {
	$c(count, (value) => {
		return console.log("comp " + value);
	}, $o);
	return "built";
});
const label = $q[0];
const scope = $q[1];
console.log(label);
$i(count, 9);
dispose2(scope);
$i(count, 10);
console.log("post");
