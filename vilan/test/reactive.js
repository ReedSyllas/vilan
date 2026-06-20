function __shared_new(value) {
	return { v: value };
}
function run_effect(e) {
	const previous = running.v;
	running.v = [ 0, e ];
	const body = e[1];
	body();
	running.v = previous;
}
function effect(body) {
	const id = next_id.v;
	next_id.v = id + 1;
	run_effect([ id, body ]);
}
function new2(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $a(self) {
	const $b = running.v;
	let $c = null;
	if ($b[0] === 0) {
		const e = $b[1];
		let already = false;
		for (const subscriber of self[1].v) {
			if (subscriber[0] === e[0]) {
				already = true;
			}
		}
		if (!(already)) {
			self[1].v.push(e);
		}
		$c = undefined;
	} else {
		$c = undefined;
	}
	$c;
	return self[0].v;
}
function $d(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		run_effect(subscriber);
	}
}
const running = __shared_new([ 1 ]);
const next_id = __shared_new(0);
const first = new2("Ada");
const last = new2("Lovelace");
effect(() => {
	console.log($a(first) + " " + $a(last));
	return;
});
effect(() => {
	console.log("first: " + $a(first));
	return;
});
console.log("---");
$d(first, "Grace");
console.log("---");
$d(last, "Hopper");
