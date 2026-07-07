function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function __json_tag(value) {
	return typeof value === "string" ? value : Object.keys(value)[0];
}
function __list_get(list, index) {
	return index >= 0 && index < list.length ? [ 0, __clone(list[index]) ] : [ 1 ];
}
function __list_pop(list) {
	return list.length === 0 ? [ 1 ] : [ 0, list.pop() ];
}
function __shared_new(value) {
	return { v: value };
}
function __try_parse_json(text) {
	try {
		return [ 0, JSON.parse(text) ];
	} catch (error) {
		return [ 1 ];
	}
}
function call(self, request) {
	return (async () => {
		return self[0](request);
	})();
}
function send(self, frame) {
	const $aP = self[0].v;
	let $aQ = null;
	if ($aP[0] === 0) {
		const handler = $aP[1];
		$aQ = handler(frame);
	} else {
		$aQ = undefined;
	}
	return $aQ;
}
function on_frame(self, handler) {
	self[1].v = [ 0, handler ];
}
function duplex_pair() {
	const slot_a = __shared_new([ 1 ]);
	const slot_b = __shared_new([ 1 ]);
	const a = [ slot_b, slot_a ];
	const b = [ slot_a, slot_b ];
	return [ a, b ];
}
function encode_request(codec, method, args) {
	const $ah = codec[0]();
	const serializer2 = $ah[0];
	const finish = $ah[1];
	serializer2[0](2);
	serializer2[1]("method");
	serializer2[9](method);
	serializer2[1]("args");
	serializer2[3](args.length);
	for (const describe of args) {
		describe(serializer2);
	}
	serializer2[4]();
	serializer2[2]();
	return finish();
}
function open_request(codec, frame) {
	const deserializer2 = codec[1](frame);
	deserializer2[0]();
	deserializer2[1]("method");
	const method = deserializer2[10]();
	deserializer2[1]("args");
	const arity = deserializer2[3]();
	return [ method, deserializer2, arity ];
}
function encode_reply(codec, outcome) {
	const $X = codec[0]();
	const serializer2 = $X[0];
	const finish = $X[1];
	const $Y = outcome;
	let $Z = null;
	if ($Y[0] === 0) {
		const describe = $Y[1];
		serializer2[5]("Success", 1);
		describe(serializer2);
		serializer2[6]();
		$Z = undefined;
	} else {
		const error = $Y[1];
		serializer2[5]("Failure", 1);
		$aa(error, serializer2);
		serializer2[6]();
		$Z = undefined;
	}
	$Z;
	return finish();
}
function respond(self, frame) {
	const request = open_request(self[0], frame);
	const $V = request[1][16]();
	let $W = null;
	if ($V[0] === 0) {
		const reason = $V[1];
		$W = [ 1, [ 1, reason ] ];
	} else {
		$W = self[1](request);
	}
	const outcome = $W;
	return encode_reply(self[0], outcome);
}
function local_rpc(protocol) {
	return [ (frame) => {
		return $ad(() => {
			return respond(protocol, frame);
		});
	} ];
}
function decode_failed(request) {
	return request[1][16]();
}
function new2() {
	return [ __shared_new([  ]) ];
}
function on(self, method, handler) {
	self[0].v.push([ method, handler ]);
	return self;
}
function handle(self, request) {
	const $T = $S($R(self[0].v, (route2) => {
		return route2[0] === request[0];
	}));
	let $U = null;
	if ($T[0] === 0) {
		const route = $T[1];
		$U = route[1](request);
	} else {
		$U = [ 1, [ 2, "unknown method: " + request[0] ] ];
	}
	return $U;
}
function into_protocol(self, codec) {
	return [ codec, (request) => {
		return handle(self, request);
	} ];
}
function encode_control(codec, kind, channel) {
	const $bd = codec[0]();
	const serializer2 = $bd[0];
	const finish = $bd[1];
	serializer2[5](kind, 1);
	serializer2[10](channel);
	serializer2[6]();
	return finish();
}
function encode_update(codec, channel, describe) {
	const $aO = codec[0]();
	const serializer2 = $aO[0];
	const finish = $aO[1];
	serializer2[5]("Update", 2);
	serializer2[10](channel);
	describe(serializer2);
	serializer2[6]();
	return finish();
}
function session_of(connection) {
	for (const entry of reactive_sessions.v) {
		const $bI = entry;
		const id = $bI[0];
		const session = $bI[1];
		if (id === connection) {
			return [ 0, session ];
		}
	}
	return [ 1 ];
}
function fresh_channel() {
	const id = next_channel.v;
	next_channel.v = id + 1;
	return id;
}
function new3(transport, codec) {
	const server = [ transport, codec, __shared_new([  ]), __shared_new([  ]) ];
	on_frame(server[0], (frame) => {
		$aM(() => {
			return receive(server, frame);
		});
		return;
	});
	return server;
}
function start(self, channel) {
	for (const entry of self[2].v) {
		const $aK = entry;
		const id = $aK[0];
		const starter = $aK[1];
		if (id === channel) {
			self[3].v.push([ channel, starter() ]);
		}
	}
}
function stop(self, channel) {
	let kept = [  ];
	for (const entry of self[3].v) {
		const $aL = entry;
		const id = $aL[0];
		const subscription = $aL[1];
		if (id === channel) {
			dispose(subscription);
		} else {
			kept.push([ id, subscription ]);
		}
	}
	self[3].v = kept;
}
function receive(self, frame) {
	const deserializer2 = self[1][1](frame);
	const tag = deserializer2[5]();
	deserializer2[6](tag, 1);
	const channel = deserializer2[11]();
	const $aG = deserializer2[16]();
	let $aH = null;
	if ($aG[0] === 0) {
		const _reason = $aG[1];
		$aH = undefined;
	} else {
		const $aI = tag;
		let $aJ = null;
		if ($aI === "Subscribe") {
			$aJ = start(self, channel);
		} else if ($aI === "Unsubscribe") {
			$aJ = stop(self, channel);
		} else {
			$aJ = undefined;
		}
		$aH = $aJ;
	}
	return $aH;
}
function new4(transport, codec) {
	const client = [ transport, codec, __shared_new([  ]) ];
	on_frame(client[0], (frame) => {
		$aM(() => {
			return receive2(client, frame);
		});
		return;
	});
	return client;
}
function receive2(self, frame) {
	const deserializer2 = self[1][1](frame);
	const tag = deserializer2[5]();
	deserializer2[6](tag, 2);
	const channel = deserializer2[11]();
	let $aW = null;
	if (tag === "Update") {
		const $aT = deserializer2[16]();
		let $aU = null;
		if ($aT[0] === 0) {
			const _reason = $aT[1];
			$aU = undefined;
		} else {
			for (const route of self[2].v) {
				const $aV = route;
				const id = $aV[0];
				const deliver = $aV[1];
				if (id === channel) {
					deliver(frame);
				}
			}
			$aU = undefined;
		}
		$aW = $aU;
	}
	return $aW;
}
function debug(self) {
	const $av = self;
	let $aw = null;
	if ($av[0] === 0) {
		const p0 = $av[1];
		$aw = "Transport(" + JSON.stringify(p0) + ")";
	} else if ($av[0] === 1) {
		const p02 = $av[1];
		$aw = "Decode(" + JSON.stringify(p02) + ")";
	} else if ($av[0] === 2) {
		const p03 = $av[1];
		$aw = "Remote(" + JSON.stringify(p03) + ")";
	} else if ($av[0] === 3) {
		const p04 = $av[1];
		$aw = "Contract(" + JSON.stringify(p04) + ")";
	} else {
		$aw = "Unauthorized";
	}
	return $aw;
}
function begin_struct(self, fields) {
	self[0](fields);
}
function field(self, name) {
	self[1](name);
}
function end_struct(self) {
	self[2]();
}
function begin_list(self, length) {
	self[3](length);
}
function end_list(self) {
	self[4]();
}
function begin_variant(self, name, arity) {
	self[5](name, arity);
}
function end_variant(self) {
	self[6]();
}
function null_value(self) {
	self[7]();
}
function some_value(self) {
	self[8]();
}
function str_value(self, value2) {
	self[9](value2);
}
function i32_value(self, value2) {
	self[10](value2);
}
function bool_value(self, value2) {
	self[13](value2);
}
function begin_struct2(self) {
	self[0]();
}
function field2(self, name) {
	self[1](name);
}
function end_struct2(self) {
	self[2]();
}
function variant_tag(self) {
	return self[5]();
}
function begin_variant2(self, name, arity) {
	self[6](name, arity);
}
function end_variant2(self) {
	self[7]();
}
function is_null(self) {
	return self[8]();
}
function null_value2(self) {
	self[9]();
}
function str_value2(self) {
	return self[10]();
}
function i32_value2(self) {
	return self[11]();
}
function bool_value2(self) {
	return self[14]();
}
function fail(self, reason) {
	self[15](reason);
}
function decode_utf8(bytes) {
	return new TextDecoder().decode(bytes);
}
function new5() {
	return [ __shared_new(""), __shared_new(false), __shared_new([  ]), __shared_new([  ]) ];
}
function value(self, text) {
	if (self[1].v) {
		self[0].v = self[0].v + ",";
	}
	self[0].v = self[0].v + text;
	self[1].v = true;
}
function open(self, opener) {
	value(self, opener);
	self[2].v.push(true);
	self[1].v = false;
}
function close(self, closer) {
	self[0].v = self[0].v + closer;
	const $p = __list_pop(self[2].v);
	let $q = null;
	if ($p[0] === 0) {
		const saved = $p[1];
		$q = saved;
	} else {
		$q = false;
	}
	self[1].v = $q;
}
function result(self) {
	return self[0].v;
}
function serializer(self) {
	return [ (fields) => {
		return begin_struct3(self, fields);
	}, (name) => {
		return field3(self, name);
	}, () => {
		return end_struct3(self);
	}, (length) => {
		return begin_list2(self, length);
	}, () => {
		return end_list2(self);
	}, (name, arity) => {
		return begin_variant3(self, name, arity);
	}, () => {
		return end_variant3(self);
	}, () => {
		return null_value3(self);
	}, () => {
		return some_value2(self);
	}, (value2) => {
		return str_value3(self, value2);
	}, (value2) => {
		return i32_value3(self, value2);
	}, (value2) => {
		return u32_value(self, value2);
	}, (value2) => {
		return f64_value(self, value2);
	}, (value2) => {
		return bool_value3(self, value2);
	} ];
}
function begin_struct3(self, fields) {
	open(self, "{");
}
function field3(self, name) {
	if (self[1].v) {
		self[0].v = self[0].v + ",";
	}
	self[0].v = self[0].v + JSON.stringify(name) + ":";
	self[1].v = false;
}
function end_struct3(self) {
	close(self, "}");
}
function begin_list2(self, length) {
	open(self, "[");
}
function end_list2(self) {
	close(self, "]");
}
function begin_variant3(self, name, arity) {
	self[3].v.push(arity);
	let $r = null;
	if (arity === 0) {
		value(self, JSON.stringify(name));
	} else {
		open(self, "{");
		self[0].v = self[0].v + JSON.stringify(name) + ":";
		self[1].v = false;
		if (arity > 1) {
			self[0].v = self[0].v + "[";
		}
		$r = undefined;
	}
	return $r;
}
function end_variant3(self) {
	const $s = __list_pop(self[3].v);
	let $t = null;
	if ($s[0] === 0) {
		const opened = $s[1];
		$t = opened;
	} else {
		$t = 0;
	}
	const arity = $t;
	if (arity > 1) {
		self[0].v = self[0].v + "]";
	}
	if (arity > 0) {
		close(self, "}");
	}
}
function null_value3(self) {
	value(self, "null");
}
function some_value2(self) {

}
function str_value3(self, value2) {
	value(self, JSON.stringify(value2));
}
function i32_value3(self, value2) {
	value(self, "" + value2);
}
function u32_value(self, value2) {
	value(self, "" + value2);
}
function f64_value(self, value2) {
	value(self, "" + value2);
}
function bool_value3(self, value2) {
	value(self, "" + value2);
}
function new6(root) {
	const stack = __shared_new([  ]);
	stack.v.push(root);
	return [ stack, __shared_new([ 1 ]) ];
}
function ok(self) {
	const $A = self[1].v;
	let $B = null;
	if ($A[0] === 0) {
		const _reason = $A[1];
		$B = false;
	} else {
		$B = true;
	}
	return $B;
}
function report(self, reason) {
	const $y = self[1].v;
	let $z = null;
	if ($y[0] === 0) {
		const _first = $y[1];
		$z = undefined;
	} else {
		self[1].v = [ 0, reason ];
		$z = undefined;
	}
	return $z;
}
function top(self) {
	let $D = null;
	if (!(ok(self)) || $C(self[0].v)) {
		$D = JSON.parse("null");
	} else {
		const values = self[0].v;
		$D = values[values.length - 1];
	}
	return $D;
}
function take(self) {
	if (!(ok(self))) {
		return JSON.parse("null");
	}
	const $F = __list_pop(self[0].v);
	let $G = null;
	if ($F[0] === 0) {
		const value2 = $F[1];
		$G = value2;
	} else {
		report(self, "unexpected end of document");
		$G = JSON.parse("null");
	}
	return $G;
}
function deserializer(self) {
	return [ () => {
		return begin_struct4(self);
	}, (name) => {
		return field4(self, name);
	}, () => {
		return end_struct4(self);
	}, () => {
		return begin_list3(self);
	}, () => {
		return end_list3(self);
	}, () => {
		return variant_tag2(self);
	}, (name, arity) => {
		return begin_variant4(self, name, arity);
	}, () => {
		return end_variant4(self);
	}, () => {
		return is_null2(self);
	}, () => {
		return null_value4(self);
	}, () => {
		return str_value4(self);
	}, () => {
		return i32_value4(self);
	}, () => {
		return u32_value2(self);
	}, () => {
		return f64_value2(self);
	}, () => {
		return bool_value4(self);
	}, (reason) => {
		return fail2(self, reason);
	}, () => {
		return failed(self);
	} ];
}
function begin_struct4(self) {

}
function field4(self, name) {
	const subject = top(self);
	let $E = null;
	if (ok(self)) {
		if (Object.hasOwn(subject, name)) {
			self[0].v.push(subject[name]);
		} else {
			report(self, "missing field \'" + name + "\'");
		}
		$E = undefined;
	}
	return $E;
}
function end_struct4(self) {
	take(self);
}
function begin_list3(self) {
	const subject = take(self);
	let $H = null;
	if (ok(self)) {
		const elements = subject;
		let index = elements.length - 1;
		while (index >= 0) {
			self[0].v.push(elements[index]);
			index = index - 1;
		}
		$H = elements.length;
	} else {
		$H = 0;
	}
	return $H;
}
function end_list3(self) {

}
function variant_tag2(self) {
	let $I = null;
	if (ok(self)) {
		$I = __json_tag(top(self));
	} else {
		$I = "";
	}
	return $I;
}
function begin_variant4(self, name, arity) {
	const subject = take(self);
	let $L = null;
	if (ok(self) && arity > 0) {
		let $K = null;
		if (Object.hasOwn(subject, name)) {
			const payload = subject[name];
			let $J = null;
			if (arity === 1) {
				self[0].v.push(payload);
			} else {
				const elements = payload;
				let index = elements.length - 1;
				while (index >= 0) {
					self[0].v.push(elements[index]);
					index = index - 1;
				}
				$J = undefined;
			}
			$K = $J;
		} else {
			report(self, "missing payload for variant \'" + name + "\'");
		}
		$L = $K;
	}
	return $L;
}
function end_variant4(self) {

}
function is_null2(self) {
	return ok(self) && top(self) === null;
}
function null_value4(self) {
	take(self);
}
function str_value4(self) {
	const value2 = take(self);
	let $M = null;
	if (ok(self)) {
		$M = String(value2);
	} else {
		$M = "";
	}
	return $M;
}
function i32_value4(self) {
	const value2 = take(self);
	let $N = null;
	if (ok(self)) {
		$N = Number(value2);
	} else {
		$N = 0;
	}
	return $N;
}
function u32_value2(self) {
	const value2 = take(self);
	let $O = null;
	if (ok(self)) {
		$O = Number(value2);
	} else {
		$O = 0;
	}
	return $O;
}
function f64_value2(self) {
	const value2 = take(self);
	let $P = null;
	if (ok(self)) {
		$P = Number(value2);
	} else {
		$P = 0.0;
	}
	return $P;
}
function bool_value4(self) {
	const value2 = take(self);
	let $Q = null;
	if (ok(self)) {
		$Q = Boolean(value2);
	} else {
		$Q = false;
	}
	return $Q;
}
function fail2(self, reason) {
	report(self, reason);
}
function failed(self) {
	return self[1].v;
}
function opened_reader(text) {
	const $w = __try_parse_json(text);
	let $x = null;
	if ($w[0] === 0) {
		const root = $w[1];
		$x = new6(root);
	} else {
		const reader = new6(JSON.parse("null"));
		report(reader, "malformed JSON");
		$x = reader;
	}
	return $x;
}
function json_codec() {
	return [ () => {
		const writer = new5();
		return [ serializer(writer), () => {
			return [ 0, result(writer) ];
		} ];
	}, (frame) => {
		const $u = frame;
		let $v = null;
		if ($u[0] === 0) {
			const text = $u[1];
			$v = deserializer(opened_reader(text));
		} else {
			const bytes = $u[1];
			$v = deserializer(opened_reader(decode_utf8(bytes)));
		}
		return $v;
	} ];
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
function flush() {
	if (!(scheduler[2].v)) {
		scheduler[2].v = true;
		let budget = 100000;
		while (!($ae(scheduler[0].v)) && budget > 0) {
			const wave = scheduler[0].v;
			scheduler[0].v = [  ];
			for (const subscriber of wave) {
				subscriber[1]();
				budget = budget - 1;
			}
		}
		scheduler[2].v = false;
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
function new7(plaintext) {
	return [ "sha256:" + plaintext ];
}
function matches(self, plaintext) {
	return self[0] === "sha256:" + plaintext;
}
function to_wire(self) {
	return [ self[0], self[1], "@" + self[1] ];
}
function lookup_user(id) {
	const $e = id;
	let $f = null;
	if ($e === 1) {
		$f = [ 0, [ 1, "ada", new7("hunter2") ] ];
	} else if ($e === 2) {
		$f = [ 0, [ 2, "bob", new7("swordfish") ] ];
	} else {
		$f = [ 1 ];
	}
	return $f;
}
function find_user(username) {
	const $bs = username;
	let $bt = null;
	if ($bs === "ada") {
		$bt = lookup_user(1);
	} else if ($bs === "bob") {
		$bt = lookup_user(2);
	} else {
		$bt = [ 1 ];
	}
	return $bt;
}
function accounts_dispatcher() {
	return on(new2(), "get_user", (request) => {
		const id = $a(request, 0);
		const $c = decode_failed(request);
		let $d = null;
		if ($c[0] === 0) {
			const reason = $c[1];
			$d = [ 1, [ 1, reason ] ];
		} else {
			const $g = lookup_user(id);
			let $h = null;
			if ($g[0] === 1) {
				$h = $g;
			} else {
				$h = [ 0, to_wire($g[1]) ];
			}
			$d = $i($h);
		}
		return $d;
	});
}
function login(self, username, password) {
	const $bu = find_user(username);
	let $bv = null;
	if ($bu[0] === 0) {
		const user = $bu[1];
		let $by = null;
		if (matches(user[2], password)) {
			self[1].v = [ 0, user[0] ];
			$bw(self[0], "online");
			$by = true;
		} else {
			$by = false;
		}
		$bv = $by;
	} else {
		$bv = false;
	}
	return $bv;
}
function whoami(self) {
	const $bB = self[1].v;
	if ($bB[0] === 1) {
		return $bB;
	}
	const id = $bB[1];
	const $bC = lookup_user(id);
	let $bD = null;
	if ($bC[0] === 1) {
		$bD = $bC;
	} else {
		$bD = [ 0, to_wire($bC[1]) ];
	}
	return $bD;
}
async function reactive_demo() {
	console.log("--- reactive: a remote Source<i32> ---");
	const $aE = duplex_pair();
	const client_end = $aE[0];
	const server_end = $aE[1];
	const counter = $aF(0);
	const server = new3(server_end, json_codec());
	const channel = $aN(server, counter);
	const remote = $aX(new4(client_end, json_codec()), channel);
	const subscription = $be(remote, (n2) => {
		console.log("count = " + n2);
		return;
	});
	$bj(counter, 1);
	$bj(counter, 2);
	$aM(() => {
		$bj(counter, 5);
		$bj(counter, 10);
		return;
	});
	const dispatcher2 = on(new2(), "add", (request) => {
		const amount2 = $a(request, 0);
		$bj(counter, $aS(counter) + amount2);
		$bj(counter, $aS(counter) + amount2);
		return $bl($aS(counter));
	});
	const rpc_transport = local_rpc(into_protocol(dispatcher2, json_codec()));
	const amount = 3;
	const added = await ($ax(rpc_transport, json_codec(), "add", [ (s) => {
		return $n(amount, s);
	} ]));
	const $bm = added;
	let $bn = null;
	if ($bm[0] === 0) {
		const n = $bm[1];
		$bn = console.log("rpc add -> " + n);
	} else {
		const error = $bm[1];
		$bn = console.log("rpc error: " + debug(error));
	}
	$bn;
	dispose(subscription);
	$bj(counter, 99);
}
async function session_demo() {
	console.log("--- session: the [service(Client)] paradigm, generated ---");
	const session = [ $bo("offline"), __shared_new([ 1 ]) ];
	const rpc_transport = local_rpc(into_protocol(dispatcher(session), json_codec()));
	const $bQ = duplex_pair();
	const client_end = $bQ[0];
	const server_end = $bQ[1];
	const status_channel = $bL(new3(server_end, json_codec()), session[0]);
	const status_mirror = $bR(new4(client_end, json_codec()), status_channel);
	const client = [ rpc_transport, json_codec(), status_mirror ];
	const watching = $bU(client[2], (s) => {
		console.log("status = " + s);
		return;
	});
	show_whoami(await ($bX(client)));
	show_login(await ($cf(client, "ada", "wrong")));
	show_login(await ($cf(client, "ada", "hunter2")));
	show_whoami(await ($bX(client)));
	dispose(watching);
}
function show_login(result2) {
	const $cm = result2;
	let $cn = null;
	if ($cm[0] === 0) {
		const ok2 = $cm[1];
		$cn = console.log("login -> " + ok2);
	} else {
		const error = $cm[1];
		$cn = console.log("login rpc error: " + debug(error));
	}
	return $cn;
}
function show_whoami(result2) {
	const $cd = result2;
	let $ce = null;
	if ($cd[0] === 0 && $cd[1][0] === 0) {
		const user = $cd[1][1];
		$ce = console.log("whoami -> " + user[1] + " (" + user[2] + ")");
	} else if ($cd[0] === 0 && $cd[1][0] === 1) {
		$ce = console.log("whoami -> not logged in");
	} else {
		const error = $cd[1];
		$ce = console.log("whoami rpc error: " + debug(error));
	}
	return $ce;
}
function show(result2) {
	const $at = result2;
	let $au = null;
	if ($at[0] === 0 && $at[1][0] === 0) {
		const user = $at[1][1];
		$au = console.log("ok: found " + user[1] + " (" + user[2] + ")");
	} else if ($at[0] === 0 && $at[1][0] === 1) {
		$au = console.log("ok: no such user");
	} else {
		const error = $at[1];
		$au = console.log("rpc error: " + debug(error));
	}
	return $au;
}
async function show_raw(transport) {
	const bogus = await ($ax(transport, json_codec(), "delete_everything", [  ]));
	const $aC = bogus;
	let $aD = null;
	if ($aC[0] === 0) {
		const value2 = $aC[1];
		$aD = console.log("raw ok: " + value2);
	} else {
		const error = $aC[1];
		$aD = console.log("raw error: " + debug(error));
	}
	return $aD;
}
function dispatcher(self) {
	return on(on(on(on(new2(), "login", (request) => {
		const username = $bp(request, 0);
		const password = $bp(request, 1);
		const $bq = decode_failed(request);
		let $br = null;
		if ($bq[0] === 0) {
			const reason = $bq[1];
			$br = [ 1, [ 1, reason ] ];
		} else {
			$br = $bz(login(self, username, password));
		}
		return $br;
	}), "whoami", (_) => {
		return $bE(whoami(self));
	}), "__contract", (_) => {
		return $bF(contract_hash(self));
	}), "__attach", (request) => {
		const connection = $a(request, 0);
		const $bG = decode_failed(request);
		let $bH = null;
		if ($bG[0] === 0) {
			const reason = $bG[1];
			$bH = [ 1, [ 1, reason ] ];
		} else {
			const $bJ = session_of(connection);
			let $bK = null;
			if ($bJ[0] === 0) {
				const session = $bJ[1];
				const channels = [ $bL(session, self[0]) ];
				$bK = $bO(channels);
			} else {
				$bK = [ 1, [ 2, "unknown connection" ] ];
			}
			$bH = $bK;
		}
		return $bH;
	});
}
function contract_hash(self) {
	return "4a4c8086";
}
function $b(deserializer2) {
	return i32_value2(deserializer2);
}
function $a(request, index) {
	return $b(request[1]);
}
function $n(self, serializer2) {
	i32_value(serializer2, self);
}
function $o(self, serializer2) {
	str_value(serializer2, self);
}
function $m(self, serializer2) {
	begin_struct(serializer2, 3);
	field(serializer2, "id");
	$n(self[0], serializer2);
	field(serializer2, "username");
	$o(self[1], serializer2);
	field(serializer2, "handle");
	$o(self[2], serializer2);
	end_struct(serializer2);
}
function $j(self, serializer2) {
	const $k = self;
	let $l = null;
	if ($k[0] === 0) {
		const value2 = $k[1];
		some_value(serializer2);
		$m(value2, serializer2);
		$l = undefined;
	} else {
		$l = null_value(serializer2);
	}
	return $l;
}
function $i(value2) {
	return [ 0, (serializer2) => {
		return $j(value2, serializer2);
	} ];
}
function $C(self) {
	return self.length === 0;
}
function $R(self, predicate) {
	let result2 = [  ];
	for (const item of self) {
		if (predicate(item)) {
			result2.push(item);
		}
	}
	return result2;
}
function $S(self) {
	return __list_get(self, 0);
}
function $aa(self, serializer2) {
	const $ab = self;
	let $ac = null;
	if ($ab[0] === 0) {
		const p0 = $ab[1];
		begin_variant(serializer2, "Transport", 1);
		$o(p0, serializer2);
		end_variant(serializer2);
		$ac = undefined;
	} else if ($ab[0] === 1) {
		const p02 = $ab[1];
		begin_variant(serializer2, "Decode", 1);
		$o(p02, serializer2);
		end_variant(serializer2);
		$ac = undefined;
	} else if ($ab[0] === 2) {
		const p03 = $ab[1];
		begin_variant(serializer2, "Remote", 1);
		$o(p03, serializer2);
		end_variant(serializer2);
		$ac = undefined;
	} else if ($ab[0] === 3) {
		const p04 = $ab[1];
		begin_variant(serializer2, "Contract", 1);
		$o(p04, serializer2);
		end_variant(serializer2);
		$ac = undefined;
	} else {
		begin_variant(serializer2, "Unauthorized", 0);
		end_variant(serializer2);
		$ac = undefined;
	}
	return $ac;
}
function $ae(self) {
	return self.length === 0;
}
function $ad(body) {
	scheduler[1].v = scheduler[1].v + 1;
	const result2 = body();
	if (scheduler[1].v === 1) {
		flush();
	}
	scheduler[1].v = scheduler[1].v - 1;
	return result2;
}
function $an(deserializer2) {
	return str_value2(deserializer2);
}
function $am(deserializer2) {
	begin_struct2(deserializer2);
	field2(deserializer2, "id");
	const id = $b(deserializer2);
	field2(deserializer2, "username");
	const username = $an(deserializer2);
	field2(deserializer2, "handle");
	const handle2 = $an(deserializer2);
	end_struct2(deserializer2);
	return [ id, username, handle2 ];
}
function $ak(deserializer2) {
	let $al = null;
	if (is_null(deserializer2)) {
		null_value2(deserializer2);
		$al = [ 1 ];
	} else {
		$al = [ 0, $am(deserializer2) ];
	}
	return $al;
}
function $aq(deserializer2) {
	const tag = variant_tag(deserializer2);
	const $ar = tag;
	let $as = null;
	if ($ar === "Transport") {
		begin_variant2(deserializer2, "Transport", 1);
		const p0 = $an(deserializer2);
		end_variant2(deserializer2);
		$as = [ 0, p0 ];
	} else if ($ar === "Decode") {
		begin_variant2(deserializer2, "Decode", 1);
		const p02 = $an(deserializer2);
		end_variant2(deserializer2);
		$as = [ 1, p02 ];
	} else if ($ar === "Remote") {
		begin_variant2(deserializer2, "Remote", 1);
		const p03 = $an(deserializer2);
		end_variant2(deserializer2);
		$as = [ 2, p03 ];
	} else if ($ar === "Contract") {
		begin_variant2(deserializer2, "Contract", 1);
		const p04 = $an(deserializer2);
		end_variant2(deserializer2);
		$as = [ 3, p04 ];
	} else if ($ar === "Unauthorized") {
		begin_variant2(deserializer2, "Unauthorized", 0);
		end_variant2(deserializer2);
		$as = [ 4 ];
	} else {
		fail(deserializer2, "unknown variant \'" + tag + "\'");
		const f0 = $an(deserializer2);
		$as = [ 0, f0 ];
	}
	return $as;
}
async function $ag(transport, codec, method, args) {
	const reply_frame = await (call(transport, encode_request(codec, method, args)));
	const deserializer2 = codec[1](reply_frame);
	const tag = deserializer2[5]();
	const $ai = tag;
	let $aj = null;
	if ($ai === "Success") {
		deserializer2[6]("Success", 1);
		const value2 = $ak(deserializer2);
		const $ao = deserializer2[16]();
		let $ap = null;
		if ($ao[0] === 1) {
			$ap = [ 0, value2 ];
		} else {
			const reason = $ao[1];
			$ap = [ 1, [ 1, reason ] ];
		}
		$aj = $ap;
	} else if ($ai === "Failure") {
		deserializer2[6]("Failure", 1);
		const error = $aq(deserializer2);
		deserializer2[7]();
		$aj = [ 1, error ];
	} else {
		$aj = [ 1, [ 1, "unrecognized reply envelope" ] ];
	}
	return $aj;
}
async function $af(self, id) {
	return await ($ag(self[0], self[1], "get_user", [ (s) => {
		return $n(id, s);
	} ]));
}
async function $ax(transport, codec, method, args) {
	const reply_frame = await (call(transport, encode_request(codec, method, args)));
	const deserializer2 = codec[1](reply_frame);
	const tag = deserializer2[5]();
	const $ay = tag;
	let $az = null;
	if ($ay === "Success") {
		deserializer2[6]("Success", 1);
		const value2 = $b(deserializer2);
		const $aA = deserializer2[16]();
		let $aB = null;
		if ($aA[0] === 1) {
			$aB = [ 0, value2 ];
		} else {
			const reason = $aA[1];
			$aB = [ 1, [ 1, reason ] ];
		}
		$az = $aB;
	} else if ($ay === "Failure") {
		deserializer2[6]("Failure", 1);
		const error = $aq(deserializer2);
		deserializer2[7]();
		$az = [ 1, error ];
	} else {
		$az = [ 1, [ 1, "unrecognized reply envelope" ] ];
	}
	return $az;
}
function $aF(value2) {
	let subscribers = [  ];
	return [ __shared_new(value2), __shared_new(subscribers) ];
}
function $aM(body) {
	scheduler[1].v = scheduler[1].v + 1;
	const result2 = body();
	if (scheduler[1].v === 1) {
		flush();
	}
	scheduler[1].v = scheduler[1].v - 1;
	return result2;
}
function $aS(self) {
	return self[0].v;
}
function $aR(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($aS(self));
		return;
	} ]);
	observer($aS(self));
	return [ self[1], id ];
}
function $aN(self, source) {
	const channel = fresh_channel();
	const transport = __clone(self[0]);
	const codec = __clone(self[1]);
	const starter = () => {
		return $aR(source, (value2) => {
			send(transport, encode_update(codec, channel, (serializer2) => {
				$n(value2, serializer2);
				return;
			}));
			return;
		});
	};
	self[2].v.push([ channel, starter ]);
	return channel;
}
function $aY(value2) {
	let subscribers = [  ];
	return [ __shared_new(value2), __shared_new(subscribers) ];
}
function $bb(self, value2) {
	self[0].v = value2;
	let $bc = null;
	if (scheduler[1].v === 0) {
		for (const subscriber of self[1].v) {
			subscriber[1]();
		}
		$bc = undefined;
	} else {
		enqueue(self[1].v);
	}
	return $bc;
}
function $aX(self, channel) {
	const cache = $aY([ 1 ]);
	const codec = __clone(self[1]);
	const deliver = (frame) => {
		const deserializer2 = codec[1](frame);
		const tag = deserializer2[5]();
		deserializer2[6](tag, 2);
		deserializer2[11]();
		const value2 = $b(deserializer2);
		const $aZ = deserializer2[16]();
		let $ba = null;
		if ($aZ[0] === 0) {
			const _reason = $aZ[1];
			$ba = undefined;
		} else {
			$ba = $bb(cache, [ 0, value2 ]);
		}
		return $ba;
	};
	self[2].v.push([ channel, deliver ]);
	return [ channel, encode_control(self[1], "Subscribe", channel), self[0], cache ];
}
function $bi(self) {
	return self[0].v;
}
function $bh(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($bi(self));
		return;
	} ]);
	observer($bi(self));
	return [ self[1], id ];
}
function $be(self, observer) {
	send(self[2], self[1]);
	return $bh(self[3], (value2) => {
		const $bf = value2;
		let $bg = null;
		if ($bf[0] === 0) {
			const present = $bf[1];
			$bg = observer(present);
		} else {
			$bg = undefined;
		}
		return $bg;
	});
}
function $bj(self, value2) {
	self[0].v = value2;
	let $bk = null;
	if (scheduler[1].v === 0) {
		for (const subscriber of self[1].v) {
			subscriber[1]();
		}
		$bk = undefined;
	} else {
		enqueue(self[1].v);
	}
	return $bk;
}
function $bl(value2) {
	return [ 0, (serializer2) => {
		return $n(value2, serializer2);
	} ];
}
function $bo(value2) {
	let subscribers = [  ];
	return [ __shared_new(value2), __shared_new(subscribers) ];
}
function $bp(request, index) {
	return $an(request[1]);
}
function $bw(self, value2) {
	self[0].v = value2;
	let $bx = null;
	if (scheduler[1].v === 0) {
		for (const subscriber of self[1].v) {
			subscriber[1]();
		}
		$bx = undefined;
	} else {
		enqueue(self[1].v);
	}
	return $bx;
}
function $bA(self, serializer2) {
	bool_value(serializer2, self);
}
function $bz(value2) {
	return [ 0, (serializer2) => {
		return $bA(value2, serializer2);
	} ];
}
function $bE(value2) {
	return [ 0, (serializer2) => {
		return $j(value2, serializer2);
	} ];
}
function $bF(value2) {
	return [ 0, (serializer2) => {
		return $o(value2, serializer2);
	} ];
}
function $bN(self) {
	return self[0].v;
}
function $bM(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($bN(self));
		return;
	} ]);
	observer($bN(self));
	return [ self[1], id ];
}
function $bL(self, source) {
	const channel = fresh_channel();
	const transport = __clone(self[0]);
	const codec = __clone(self[1]);
	const starter = () => {
		return $bM(source, (value2) => {
			send(transport, encode_update(codec, channel, (serializer2) => {
				$o(value2, serializer2);
				return;
			}));
			return;
		});
	};
	self[2].v.push([ channel, starter ]);
	return channel;
}
function $bP(self, serializer2) {
	begin_list(serializer2, self.length);
	for (const element of self) {
		$n(element, serializer2);
	}
	end_list(serializer2);
}
function $bO(value2) {
	return [ 0, (serializer2) => {
		return $bP(value2, serializer2);
	} ];
}
function $bR(self, channel) {
	const cache = $aY([ 1 ]);
	const codec = __clone(self[1]);
	const deliver = (frame) => {
		const deserializer2 = codec[1](frame);
		const tag = deserializer2[5]();
		deserializer2[6](tag, 2);
		deserializer2[11]();
		const value2 = $an(deserializer2);
		const $bS = deserializer2[16]();
		let $bT = null;
		if ($bS[0] === 0) {
			const _reason = $bS[1];
			$bT = undefined;
		} else {
			$bT = $bb(cache, [ 0, value2 ]);
		}
		return $bT;
	};
	self[2].v.push([ channel, deliver ]);
	return [ channel, encode_control(self[1], "Subscribe", channel), self[0], cache ];
}
function $bU(self, observer) {
	send(self[2], self[1]);
	return $bh(self[3], (value2) => {
		const $bV = value2;
		let $bW = null;
		if ($bV[0] === 0) {
			const present = $bV[1];
			$bW = observer(present);
		} else {
			$bW = undefined;
		}
		return $bW;
	});
}
async function $bY(transport, codec, method, args) {
	const reply_frame = await (call(transport, encode_request(codec, method, args)));
	const deserializer2 = codec[1](reply_frame);
	const tag = deserializer2[5]();
	const $bZ = tag;
	let $ca = null;
	if ($bZ === "Success") {
		deserializer2[6]("Success", 1);
		const value2 = $ak(deserializer2);
		const $cb = deserializer2[16]();
		let $cc = null;
		if ($cb[0] === 1) {
			$cc = [ 0, value2 ];
		} else {
			const reason = $cb[1];
			$cc = [ 1, [ 1, reason ] ];
		}
		$ca = $cc;
	} else if ($bZ === "Failure") {
		deserializer2[6]("Failure", 1);
		const error = $aq(deserializer2);
		deserializer2[7]();
		$ca = [ 1, error ];
	} else {
		$ca = [ 1, [ 1, "unrecognized reply envelope" ] ];
	}
	return $ca;
}
async function $bX(self) {
	return await ($bY(self[0], self[1], "whoami", [  ]));
}
function $cj(deserializer2) {
	return bool_value2(deserializer2);
}
async function $cg(transport, codec, method, args) {
	const reply_frame = await (call(transport, encode_request(codec, method, args)));
	const deserializer2 = codec[1](reply_frame);
	const tag = deserializer2[5]();
	const $ch = tag;
	let $ci = null;
	if ($ch === "Success") {
		deserializer2[6]("Success", 1);
		const value2 = $cj(deserializer2);
		const $ck = deserializer2[16]();
		let $cl = null;
		if ($ck[0] === 1) {
			$cl = [ 0, value2 ];
		} else {
			const reason = $ck[1];
			$cl = [ 1, [ 1, reason ] ];
		}
		$ci = $cl;
	} else if ($ch === "Failure") {
		deserializer2[6]("Failure", 1);
		const error = $aq(deserializer2);
		deserializer2[7]();
		$ci = [ 1, error ];
	} else {
		$ci = [ 1, [ 1, "unrecognized reply envelope" ] ];
	}
	return $ci;
}
async function $cf(self, username, password) {
	return await ($cg(self[0], self[1], "login", [ (serializer2) => {
		return $o(username, serializer2);
	}, (serializer2) => {
		return $o(password, serializer2);
	} ]));
}
const reactive_sessions = __shared_new([  ]);
const next_channel = __shared_new(0);
const next_subscriber_id = __shared_new(0);
const scheduler = [ __shared_new([  ]), __shared_new(0), __shared_new(false) ];
(async () => {
	const transport = local_rpc(into_protocol(accounts_dispatcher(), json_codec()));
	const client = [ transport, json_codec() ];
	show(await ($af(client, 1)));
	show(await ($af(client, 9)));
	await (show_raw(transport));
	await (reactive_demo());
	await (session_demo());
})();
