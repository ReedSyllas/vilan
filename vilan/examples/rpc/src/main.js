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
function new2() {
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
		return begin_struct(self, fields);
	}, (name) => {
		return field(self, name);
	}, () => {
		return end_struct(self);
	}, (length) => {
		return begin_list(self, length);
	}, () => {
		return end_list(self);
	}, (name, arity) => {
		return begin_variant(self, name, arity);
	}, () => {
		return end_variant(self);
	}, () => {
		return null_value(self);
	}, () => {
		return some_value(self);
	}, (value2) => {
		return str_value(self, value2);
	}, (value2) => {
		return i32_value(self, value2);
	}, (value2) => {
		return u32_value(self, value2);
	}, (value2) => {
		return f64_value(self, value2);
	}, (value2) => {
		return bool_value(self, value2);
	} ];
}
function begin_struct(self, fields) {
	open(self, "{");
}
function field(self, name) {
	if (self[1].v) {
		self[0].v = self[0].v + ",";
	}
	self[0].v = self[0].v + JSON.stringify(name) + ":";
	self[1].v = false;
}
function end_struct(self) {
	close(self, "}");
}
function begin_list(self, length) {
	open(self, "[");
}
function end_list(self) {
	close(self, "]");
}
function begin_variant(self, name, arity) {
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
function end_variant(self) {
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
function null_value(self) {
	value(self, "null");
}
function some_value(self) {

}
function str_value(self, value2) {
	value(self, JSON.stringify(value2));
}
function i32_value(self, value2) {
	value(self, "" + value2);
}
function u32_value(self, value2) {
	value(self, "" + value2);
}
function f64_value(self, value2) {
	value(self, "" + value2);
}
function bool_value(self, value2) {
	value(self, "" + value2);
}
function new3(root) {
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
		return begin_struct2(self);
	}, (name) => {
		return field2(self, name);
	}, () => {
		return end_struct2(self);
	}, () => {
		return begin_list2(self);
	}, () => {
		return end_list2(self);
	}, () => {
		return variant_tag(self);
	}, (name, arity) => {
		return begin_variant2(self, name, arity);
	}, () => {
		return end_variant2(self);
	}, () => {
		return is_null(self);
	}, () => {
		return null_value2(self);
	}, () => {
		return str_value2(self);
	}, () => {
		return i32_value2(self);
	}, () => {
		return u32_value2(self);
	}, () => {
		return f64_value2(self);
	}, () => {
		return bool_value2(self);
	}, (reason) => {
		return fail(self, reason);
	}, () => {
		return failed(self);
	} ];
}
function begin_struct2(self) {

}
function field2(self, name) {
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
function end_struct2(self) {
	take(self);
}
function begin_list2(self) {
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
function end_list2(self) {

}
function variant_tag(self) {
	let $I = null;
	if (ok(self)) {
		$I = __json_tag(top(self));
	} else {
		$I = "";
	}
	return $I;
}
function begin_variant2(self, name, arity) {
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
function end_variant2(self) {

}
function is_null(self) {
	return ok(self) && top(self) === null;
}
function null_value2(self) {
	take(self);
}
function str_value2(self) {
	const value2 = take(self);
	let $M = null;
	if (ok(self)) {
		$M = String(value2);
	} else {
		$M = "";
	}
	return $M;
}
function i32_value2(self) {
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
function bool_value2(self) {
	const value2 = take(self);
	let $Q = null;
	if (ok(self)) {
		$Q = Boolean(value2);
	} else {
		$Q = false;
	}
	return $Q;
}
function fail(self, reason) {
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
		$x = new3(root);
	} else {
		const reader = new3(JSON.parse("null"));
		report(reader, "malformed JSON");
		$x = reader;
	}
	return $x;
}
function json_codec() {
	return [ () => {
		const writer = new2();
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
function begin_struct3(self, fields) {
	self[0](fields);
}
function field3(self, name) {
	self[1](name);
}
function end_struct3(self) {
	self[2]();
}
function begin_list3(self, length) {
	self[3](length);
}
function end_list3(self) {
	self[4]();
}
function begin_variant3(self, name, arity) {
	self[5](name, arity);
}
function end_variant3(self) {
	self[6]();
}
function null_value3(self) {
	self[7]();
}
function some_value2(self) {
	self[8]();
}
function str_value3(self, value2) {
	self[9](value2);
}
function i32_value3(self, value2) {
	self[10](value2);
}
function bool_value3(self, value2) {
	self[13](value2);
}
function begin_struct4(self) {
	self[0]();
}
function field4(self, name) {
	self[1](name);
}
function end_struct4(self) {
	self[2]();
}
function variant_tag2(self) {
	return self[5]();
}
function begin_variant4(self, name, arity) {
	self[6](name, arity);
}
function end_variant4(self) {
	self[7]();
}
function is_null2(self) {
	return self[8]();
}
function null_value4(self) {
	self[9]();
}
function str_value4(self) {
	return self[10]();
}
function i32_value4(self) {
	return self[11]();
}
function bool_value4(self) {
	return self[14]();
}
function fail2(self, reason) {
	self[15](reason);
}
function decode_utf8(bytes) {
	return new TextDecoder().decode(bytes);
}
function call(self, request) {
	return (async () => {
		return self[0](request);
	})();
}
function send(self, frame) {
	const $be = self[0].v;
	let $bf = null;
	if ($be[0] === 0) {
		const handler = $be[1];
		$bf = handler(frame);
	} else {
		$bf = undefined;
	}
	return $bf;
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
	const $am = codec[0]();
	const serializer2 = $am[0];
	const finish = $am[1];
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
	const $Z = codec[0]();
	const serializer2 = $Z[0];
	const finish = $Z[1];
	const $aa = outcome;
	let $ab = null;
	if ($aa[0] === 0) {
		const describe = $aa[1];
		serializer2[5]("Success", 1);
		describe(serializer2);
		serializer2[6]();
		$ab = undefined;
	} else {
		const error = $aa[1];
		serializer2[5]("Failure", 1);
		$ac(error, serializer2);
		serializer2[6]();
		$ab = undefined;
	}
	$ab;
	return finish();
}
function respond(self, frame) {
	const request = open_request(self[0], frame);
	const $X = request[1][16]();
	let $Y = null;
	if ($X[0] === 0) {
		const reason = $X[1];
		$Y = [ 1, [ 1, reason ] ];
	} else {
		$Y = self[1](request);
	}
	const outcome = $Y;
	return encode_reply(self[0], outcome);
}
function local_rpc(protocol, $V) {
	return [ (frame) => {
		return $af(($W) => {
			return respond(protocol, frame);
		}, $V);
	} ];
}
function decode_failed(request) {
	return request[1][16]();
}
function new4() {
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
	const $bA = codec[0]();
	const serializer2 = $bA[0];
	const finish = $bA[1];
	serializer2[5](kind, 1);
	serializer2[10](channel);
	serializer2[6]();
	return finish();
}
function encode_update(codec, channel, describe) {
	const $bd = codec[0]();
	const serializer2 = $bd[0];
	const finish = $bd[1];
	serializer2[5]("Update", 2);
	serializer2[10](channel);
	describe(serializer2);
	serializer2[6]();
	return finish();
}
function session_of(connection) {
	for (const entry of reactive_sessions.v) {
		const $cr = entry;
		const id = $cr[0];
		const session = $cr[1];
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
function new5(transport, codec, $aM) {
	const server = [ transport, codec, __shared_new([  ]), __shared_new([  ]) ];
	on_frame(server[0], (frame) => {
		$aZ(($aN) => {
			return receive(server, frame, [ 0, $aN ]);
		}, $aM);
		return;
	});
	return server;
}
function start(self, channel) {
	for (const entry of self[2].v) {
		const $aT = entry;
		const id = $aT[0];
		const starter = $aT[1];
		if (id === channel) {
			self[3].v.push([ channel, starter() ]);
		}
	}
}
function stop(self, channel, $aU) {
	let kept = [  ];
	for (const entry of self[3].v) {
		const $aV = entry;
		const id = $aV[0];
		const subscription = $aV[1];
		if (id === channel) {
			dispose(subscription, $aU);
		} else {
			kept.push([ id, subscription ]);
		}
	}
	self[3].v = kept;
}
function receive(self, frame, $aO) {
	const deserializer2 = self[1][1](frame);
	const tag = deserializer2[5]();
	deserializer2[6](tag, 1);
	const channel = deserializer2[11]();
	const $aP = deserializer2[16]();
	let $aQ = null;
	if ($aP[0] === 0) {
		const _reason = $aP[1];
		$aQ = undefined;
	} else {
		const $aR = tag;
		let $aS = null;
		if ($aR === "Subscribe") {
			$aS = start(self, channel);
		} else if ($aR === "Unsubscribe") {
			$aS = stop(self, channel, $aO);
		} else {
			$aS = undefined;
		}
		$aQ = $aS;
	}
	return $aQ;
}
function new6(transport, codec, $bi) {
	const client = [ transport, codec, __shared_new([  ]) ];
	on_frame(client[0], (frame) => {
		$aZ(($bj) => {
			return receive2(client, frame);
		}, $bi);
		return;
	});
	return client;
}
function receive2(self, frame) {
	const deserializer2 = self[1][1](frame);
	const tag = deserializer2[5]();
	deserializer2[6](tag, 2);
	const channel = deserializer2[11]();
	let $bn = null;
	if (tag === "Update") {
		const $bk = deserializer2[16]();
		let $bl = null;
		if ($bk[0] === 0) {
			const _reason = $bk[1];
			$bl = undefined;
		} else {
			for (const route of self[2].v) {
				const $bm = route;
				const id = $bm[0];
				const deliver = $bm[1];
				if (id === channel) {
					deliver(frame);
				}
			}
			$bl = undefined;
		}
		$bn = $bl;
	}
	return $bn;
}
function debug(self) {
	const $aA = self;
	let $aB = null;
	if ($aA[0] === 0) {
		const p0 = $aA[1];
		$aB = "Transport(" + JSON.stringify(p0) + ")";
	} else if ($aA[0] === 1) {
		const p02 = $aA[1];
		$aB = "Decode(" + JSON.stringify(p02) + ")";
	} else if ($aA[0] === 2) {
		const p03 = $aA[1];
		$aB = "Remote(" + JSON.stringify(p03) + ")";
	} else if ($aA[0] === 3) {
		const p04 = $aA[1];
		$aB = "Contract(" + JSON.stringify(p04) + ")";
	} else {
		$aB = "Unauthorized";
	}
	return $aB;
}
function fresh_id() {
	const id = next_subscriber_id.v;
	next_subscriber_id.v = id + 1;
	return id;
}
function new7() {
	return [ __shared_new([  ]), __shared_new(false) ];
}
function enqueue(turn, subscribers) {
	for (const subscriber of subscribers) {
		let seen = false;
		for (const queued of turn[0].v) {
			if (queued[0] === subscriber[0]) {
				seen = true;
			}
		}
		if (!(seen)) {
			turn[0].v.push(subscriber);
		}
	}
}
function drain(turn) {
	if (!(turn[1].v)) {
		turn[1].v = true;
		draining_turns.v.push(turn);
		let budget = 100000;
		while (!($aj(turn[0].v)) && budget > 0) {
			const wave = turn[0].v;
			turn[0].v = [  ];
			for (const subscriber of wave) {
				subscriber[1]();
				budget = budget - 1;
			}
		}
		__list_pop(draining_turns.v);
		turn[1].v = false;
	}
}
function dispose(self, $aW) {
	let kept = [  ];
	for (const subscriber of self[0].v) {
		if (subscriber[0] !== self[1]) {
			kept.push(subscriber);
		}
	}
	self[0].v = kept;
	const $aX = $aW;
	let $aY = null;
	if ($aX[0] === 0) {
		const turn = $aX[1];
		let kept_pending = [  ];
		for (const subscriber2 of turn[0].v) {
			if (subscriber2[0] !== self[1]) {
				kept_pending.push(subscriber2);
			}
		}
		turn[0].v = kept_pending;
		$aY = undefined;
	} else {
		$aY = undefined;
	}
	return $aY;
}
function new8(plaintext) {
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
		$f = [ 0, [ 1, "ada", new8("hunter2") ] ];
	} else if ($e === 2) {
		$f = [ 0, [ 2, "bob", new8("swordfish") ] ];
	} else {
		$f = [ 1 ];
	}
	return $f;
}
function find_user(username) {
	const $bW = username;
	let $bX = null;
	if ($bW === "ada") {
		$bX = lookup_user(1);
	} else if ($bW === "bob") {
		$bX = lookup_user(2);
	} else {
		$bX = [ 1 ];
	}
	return $bX;
}
function accounts_dispatcher() {
	return on(new4(), "get_user", (request) => {
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
function login(self, username, password, $bV) {
	const $bY = find_user(username);
	let $bZ = null;
	if ($bY[0] === 0) {
		const user = $bY[1];
		let $cf = null;
		if (matches(user[2], password)) {
			self[1].v = [ 0, user[0] ];
			$ca(self[0], "online", $bV);
			$cf = true;
		} else {
			$cf = false;
		}
		$bZ = $cf;
	} else {
		$bZ = false;
	}
	return $bZ;
}
function whoami(self) {
	const $cj = self[1].v;
	if ($cj[0] === 1) {
		return $cj;
	}
	const id = $cj[1];
	const $ck = lookup_user(id);
	let $cl = null;
	if ($ck[0] === 1) {
		$cl = $ck;
	} else {
		$cl = [ 0, to_wire($ck[1]) ];
	}
	return $cl;
}
async function reactive_demo($aJ) {
	console.log("--- reactive: a remote Source<i32> ---");
	const $aK = duplex_pair();
	const client_end = $aK[0];
	const server_end = $aK[1];
	const counter = $aL(0);
	const server = new5(server_end, json_codec(), $aJ);
	const channel = $bc(server, counter);
	const remote = $bo(new6(client_end, json_codec(), $aJ), channel, $aJ);
	const subscription = $bB(remote, (n2) => {
		console.log("count = " + n2);
		return;
	});
	$bG(counter, 1, $aJ);
	$bG(counter, 2, $aJ);
	$aZ(($bL) => {
		$bG(counter, 5, [ 0, $bL ]);
		$bG(counter, 10, [ 0, $bL ]);
		return;
	}, $aJ);
	const dispatcher2 = on(new4(), "add", (request) => {
		const amount2 = $a(request, 0);
		$bG(counter, $bh(counter) + amount2, $aJ);
		$bG(counter, $bh(counter) + amount2, $aJ);
		return $bM($bh(counter));
	});
	const rpc_transport = local_rpc(into_protocol(dispatcher2, json_codec()), $aJ);
	const amount = 3;
	const added = await ($aC(rpc_transport, json_codec(), "add", [ (s) => {
		return $n(amount, s);
	} ]));
	const $bN = added;
	let $bO = null;
	if ($bN[0] === 0) {
		const n = $bN[1];
		$bO = console.log("rpc add -> " + n);
	} else {
		const error = $bN[1];
		$bO = console.log("rpc error: " + debug(error));
	}
	$bO;
	dispose(subscription, $aJ);
	$bG(counter, 99, $aJ);
}
async function session_demo($bP) {
	console.log("--- session: the [service(Client)] paradigm, generated ---");
	const session = [ $bQ("offline"), __shared_new([ 1 ]) ];
	const rpc_transport = local_rpc(into_protocol(dispatcher(session), json_codec()), $bP);
	const $cz = duplex_pair();
	const client_end = $cz[0];
	const server_end = $cz[1];
	const status_channel = $cu(new5(server_end, json_codec(), $bP), session[0]);
	const status_mirror = $cA(new6(client_end, json_codec(), $bP), status_channel, $bP);
	const client = [ rpc_transport, json_codec(), status_mirror ];
	const watching = $cD(client[2], (s) => {
		console.log("status = " + s);
		return;
	});
	show_whoami(await ($cG(client)));
	show_login(await ($cO(client, "ada", "wrong")));
	show_login(await ($cO(client, "ada", "hunter2")));
	show_whoami(await ($cG(client)));
	dispose(watching, $bP);
}
function show_login(result2) {
	const $cV = result2;
	let $cW = null;
	if ($cV[0] === 0) {
		const ok2 = $cV[1];
		$cW = console.log("login -> " + ok2);
	} else {
		const error = $cV[1];
		$cW = console.log("login rpc error: " + debug(error));
	}
	return $cW;
}
function show_whoami(result2) {
	const $cM = result2;
	let $cN = null;
	if ($cM[0] === 0 && $cM[1][0] === 0) {
		const user = $cM[1][1];
		$cN = console.log("whoami -> " + user[1] + " (" + user[2] + ")");
	} else if ($cM[0] === 0 && $cM[1][0] === 1) {
		$cN = console.log("whoami -> not logged in");
	} else {
		const error = $cM[1];
		$cN = console.log("whoami rpc error: " + debug(error));
	}
	return $cN;
}
function show(result2) {
	const $ay = result2;
	let $az = null;
	if ($ay[0] === 0 && $ay[1][0] === 0) {
		const user = $ay[1][1];
		$az = console.log("ok: found " + user[1] + " (" + user[2] + ")");
	} else if ($ay[0] === 0 && $ay[1][0] === 1) {
		$az = console.log("ok: no such user");
	} else {
		const error = $ay[1];
		$az = console.log("rpc error: " + debug(error));
	}
	return $az;
}
async function show_raw(transport) {
	const bogus = await ($aC(transport, json_codec(), "delete_everything", [  ]));
	const $aH = bogus;
	let $aI = null;
	if ($aH[0] === 0) {
		const value2 = $aH[1];
		$aI = console.log("raw ok: " + value2);
	} else {
		const error = $aH[1];
		$aI = console.log("raw error: " + debug(error));
	}
	return $aI;
}
function dispatcher(self) {
	return on(on(on(on(new4(), "login", (request) => {
		return $ci([ 0 ], ($bR) => {
			const username = $bS(request, 0);
			const password = $bS(request, 1);
			const $bT = decode_failed(request);
			let $bU = null;
			if ($bT[0] === 0) {
				const reason = $bT[1];
				$bU = [ 1, [ 1, reason ] ];
			} else {
				$bU = $cg(login(self, username, password, [ 0, $bR ]));
			}
			return $bU;
		});
	}), "whoami", (_) => {
		return $cm(whoami(self));
	}), "__contract", (_) => {
		return $cn(contract_hash(self));
	}), "__attach", (request) => {
		return $ci([ 0 ], ($co) => {
			const connection = $a(request, 0);
			const $cp = decode_failed(request);
			let $cq = null;
			if ($cp[0] === 0) {
				const reason = $cp[1];
				$cq = [ 1, [ 1, reason ] ];
			} else {
				const $cs = session_of(connection);
				let $ct = null;
				if ($cs[0] === 0) {
					const session = $cs[1];
					const channels = [ $cu(session, self[0]) ];
					$ct = $cx(channels);
				} else {
					$ct = [ 1, [ 2, "unknown connection" ] ];
				}
				$cq = $ct;
			}
			return $cq;
		});
	});
}
function contract_hash(self) {
	return "4a4c8086";
}
function $b(deserializer2) {
	return i32_value4(deserializer2);
}
function $a(request, index) {
	return $b(request[1]);
}
function $n(self, serializer2) {
	i32_value3(serializer2, self);
}
function $o(self, serializer2) {
	str_value3(serializer2, self);
}
function $m(self, serializer2) {
	begin_struct3(serializer2, 3);
	field3(serializer2, "id");
	$n(self[0], serializer2);
	field3(serializer2, "username");
	$o(self[1], serializer2);
	field3(serializer2, "handle");
	$o(self[2], serializer2);
	end_struct3(serializer2);
}
function $j(self, serializer2) {
	const $k = self;
	let $l = null;
	if ($k[0] === 0) {
		const value2 = $k[1];
		some_value2(serializer2);
		$m(value2, serializer2);
		$l = undefined;
	} else {
		$l = null_value3(serializer2);
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
function $ac(self, serializer2) {
	const $ad = self;
	let $ae = null;
	if ($ad[0] === 0) {
		const p0 = $ad[1];
		begin_variant3(serializer2, "Transport", 1);
		$o(p0, serializer2);
		end_variant3(serializer2);
		$ae = undefined;
	} else if ($ad[0] === 1) {
		const p02 = $ad[1];
		begin_variant3(serializer2, "Decode", 1);
		$o(p02, serializer2);
		end_variant3(serializer2);
		$ae = undefined;
	} else if ($ad[0] === 2) {
		const p03 = $ad[1];
		begin_variant3(serializer2, "Remote", 1);
		$o(p03, serializer2);
		end_variant3(serializer2);
		$ae = undefined;
	} else if ($ad[0] === 3) {
		const p04 = $ad[1];
		begin_variant3(serializer2, "Contract", 1);
		$o(p04, serializer2);
		end_variant3(serializer2);
		$ae = undefined;
	} else {
		begin_variant3(serializer2, "Unauthorized", 0);
		end_variant3(serializer2);
		$ae = undefined;
	}
	return $ae;
}
function $aj(self) {
	return self.length === 0;
}
function $af(body, $ag) {
	const $ah = $ag;
	let $ai = null;
	if ($ah[0] === 0) {
		const current = $ah[1];
		$ai = body(current);
	} else {
		const fresh = new7();
		const result2 = body(fresh);
		drain(fresh);
		$ai = result2;
	}
	return $ai;
}
function $as(deserializer2) {
	return str_value4(deserializer2);
}
function $ar(deserializer2) {
	begin_struct4(deserializer2);
	field4(deserializer2, "id");
	const id = $b(deserializer2);
	field4(deserializer2, "username");
	const username = $as(deserializer2);
	field4(deserializer2, "handle");
	const handle2 = $as(deserializer2);
	end_struct4(deserializer2);
	return [ id, username, handle2 ];
}
function $ap(deserializer2) {
	let $aq = null;
	if (is_null2(deserializer2)) {
		null_value4(deserializer2);
		$aq = [ 1 ];
	} else {
		$aq = [ 0, $ar(deserializer2) ];
	}
	return $aq;
}
function $av(deserializer2) {
	const tag = variant_tag2(deserializer2);
	const $aw = tag;
	let $ax = null;
	if ($aw === "Transport") {
		begin_variant4(deserializer2, "Transport", 1);
		const p0 = $as(deserializer2);
		end_variant4(deserializer2);
		$ax = [ 0, p0 ];
	} else if ($aw === "Decode") {
		begin_variant4(deserializer2, "Decode", 1);
		const p02 = $as(deserializer2);
		end_variant4(deserializer2);
		$ax = [ 1, p02 ];
	} else if ($aw === "Remote") {
		begin_variant4(deserializer2, "Remote", 1);
		const p03 = $as(deserializer2);
		end_variant4(deserializer2);
		$ax = [ 2, p03 ];
	} else if ($aw === "Contract") {
		begin_variant4(deserializer2, "Contract", 1);
		const p04 = $as(deserializer2);
		end_variant4(deserializer2);
		$ax = [ 3, p04 ];
	} else if ($aw === "Unauthorized") {
		begin_variant4(deserializer2, "Unauthorized", 0);
		end_variant4(deserializer2);
		$ax = [ 4 ];
	} else {
		fail2(deserializer2, "unknown variant \'" + tag + "\'");
		const f0 = $as(deserializer2);
		$ax = [ 0, f0 ];
	}
	return $ax;
}
async function $al(transport, codec, method, args) {
	const reply_frame = await (call(transport, encode_request(codec, method, args)));
	const deserializer2 = codec[1](reply_frame);
	const tag = deserializer2[5]();
	const $an = tag;
	let $ao = null;
	if ($an === "Success") {
		deserializer2[6]("Success", 1);
		const value2 = $ap(deserializer2);
		const $at = deserializer2[16]();
		let $au = null;
		if ($at[0] === 1) {
			$au = [ 0, value2 ];
		} else {
			const reason = $at[1];
			$au = [ 1, [ 1, reason ] ];
		}
		$ao = $au;
	} else if ($an === "Failure") {
		deserializer2[6]("Failure", 1);
		const error = $av(deserializer2);
		deserializer2[7]();
		$ao = [ 1, error ];
	} else {
		$ao = [ 1, [ 1, "unrecognized reply envelope" ] ];
	}
	return $ao;
}
async function $ak(self, id) {
	return await ($al(self[0], self[1], "get_user", [ (s) => {
		return $n(id, s);
	} ]));
}
async function $aC(transport, codec, method, args) {
	const reply_frame = await (call(transport, encode_request(codec, method, args)));
	const deserializer2 = codec[1](reply_frame);
	const tag = deserializer2[5]();
	const $aD = tag;
	let $aE = null;
	if ($aD === "Success") {
		deserializer2[6]("Success", 1);
		const value2 = $b(deserializer2);
		const $aF = deserializer2[16]();
		let $aG = null;
		if ($aF[0] === 1) {
			$aG = [ 0, value2 ];
		} else {
			const reason = $aF[1];
			$aG = [ 1, [ 1, reason ] ];
		}
		$aE = $aG;
	} else if ($aD === "Failure") {
		deserializer2[6]("Failure", 1);
		const error = $av(deserializer2);
		deserializer2[7]();
		$aE = [ 1, error ];
	} else {
		$aE = [ 1, [ 1, "unrecognized reply envelope" ] ];
	}
	return $aE;
}
function $aL(value2) {
	let subscribers = [  ];
	return [ __shared_new(value2), __shared_new(subscribers) ];
}
function $aZ(body, $ag) {
	const $ba = $ag;
	let $bb = null;
	if ($ba[0] === 0) {
		const current = $ba[1];
		$bb = body(current);
	} else {
		const fresh = new7();
		const result2 = body(fresh);
		drain(fresh);
		$bb = result2;
	}
	return $bb;
}
function $bh(self) {
	return self[0].v;
}
function $bg(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($bh(self));
		return;
	} ]);
	observer($bh(self));
	return [ self[1], id ];
}
function $bc(self, source) {
	const channel = fresh_channel();
	const transport = __clone(self[0]);
	const codec = __clone(self[1]);
	const starter = () => {
		return $bg(source, (value2) => {
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
function $bq(value2) {
	let subscribers = [  ];
	return [ __shared_new(value2), __shared_new(subscribers) ];
}
function $bx(self) {
	return __list_get(self, self.length - 1);
}
function $bt(self, value2, $bu) {
	self[0].v = value2;
	const $bv = $bu;
	let $bw = null;
	if ($bv[0] === 0) {
		const turn = $bv[1];
		$bw = enqueue(turn, self[1].v);
	} else {
		const $by = $bx(draining_turns.v);
		let $bz = null;
		if ($by[0] === 0) {
			const draining = $by[1];
			$bz = enqueue(draining, self[1].v);
		} else {
			for (const subscriber of self[1].v) {
				subscriber[1]();
			}
			$bz = undefined;
		}
		$bw = $bz;
	}
	return $bw;
}
function $bo(self, channel, $bp) {
	const cache = $bq([ 1 ]);
	const codec = __clone(self[1]);
	const deliver = (frame) => {
		const deserializer2 = codec[1](frame);
		const tag = deserializer2[5]();
		deserializer2[6](tag, 2);
		deserializer2[11]();
		const value2 = $b(deserializer2);
		const $br = deserializer2[16]();
		let $bs = null;
		if ($br[0] === 0) {
			const _reason = $br[1];
			$bs = undefined;
		} else {
			$bs = $bt(cache, [ 0, value2 ], $bp);
		}
		return $bs;
	};
	self[2].v.push([ channel, deliver ]);
	return [ channel, encode_control(self[1], "Subscribe", channel), self[0], cache ];
}
function $bF(self) {
	return self[0].v;
}
function $bE(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($bF(self));
		return;
	} ]);
	observer($bF(self));
	return [ self[1], id ];
}
function $bB(self, observer) {
	send(self[2], self[1]);
	return $bE(self[3], (value2) => {
		const $bC = value2;
		let $bD = null;
		if ($bC[0] === 0) {
			const present = $bC[1];
			$bD = observer(present);
		} else {
			$bD = undefined;
		}
		return $bD;
	});
}
function $bG(self, value2, $bu) {
	self[0].v = value2;
	const $bH = $bu;
	let $bI = null;
	if ($bH[0] === 0) {
		const turn = $bH[1];
		$bI = enqueue(turn, self[1].v);
	} else {
		const $bJ = $bx(draining_turns.v);
		let $bK = null;
		if ($bJ[0] === 0) {
			const draining = $bJ[1];
			$bK = enqueue(draining, self[1].v);
		} else {
			for (const subscriber of self[1].v) {
				subscriber[1]();
			}
			$bK = undefined;
		}
		$bI = $bK;
	}
	return $bI;
}
function $bM(value2) {
	return [ 0, (serializer2) => {
		return $n(value2, serializer2);
	} ];
}
function $bQ(value2) {
	let subscribers = [  ];
	return [ __shared_new(value2), __shared_new(subscribers) ];
}
function $bS(request, index) {
	return $as(request[1]);
}
function $ca(self, value2, $bu) {
	self[0].v = value2;
	const $cb = $bu;
	let $cc = null;
	if ($cb[0] === 0) {
		const turn = $cb[1];
		$cc = enqueue(turn, self[1].v);
	} else {
		const $cd = $bx(draining_turns.v);
		let $ce = null;
		if ($cd[0] === 0) {
			const draining = $cd[1];
			$ce = enqueue(draining, self[1].v);
		} else {
			for (const subscriber of self[1].v) {
				subscriber[1]();
			}
			$ce = undefined;
		}
		$cc = $ce;
	}
	return $cc;
}
function $ch(self, serializer2) {
	bool_value3(serializer2, self);
}
function $cg(value2) {
	return [ 0, (serializer2) => {
		return $ch(value2, serializer2);
	} ];
}
function $ci(policy, body) {
	const fresh = new7();
	const result2 = body(fresh);
	drain(fresh);
	return result2;
}
function $cm(value2) {
	return [ 0, (serializer2) => {
		return $j(value2, serializer2);
	} ];
}
function $cn(value2) {
	return [ 0, (serializer2) => {
		return $o(value2, serializer2);
	} ];
}
function $cw(self) {
	return self[0].v;
}
function $cv(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($cw(self));
		return;
	} ]);
	observer($cw(self));
	return [ self[1], id ];
}
function $cu(self, source) {
	const channel = fresh_channel();
	const transport = __clone(self[0]);
	const codec = __clone(self[1]);
	const starter = () => {
		return $cv(source, (value2) => {
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
function $cy(self, serializer2) {
	begin_list3(serializer2, self.length);
	for (const element of self) {
		$n(element, serializer2);
	}
	end_list3(serializer2);
}
function $cx(value2) {
	return [ 0, (serializer2) => {
		return $cy(value2, serializer2);
	} ];
}
function $cA(self, channel, $bp) {
	const cache = $bq([ 1 ]);
	const codec = __clone(self[1]);
	const deliver = (frame) => {
		const deserializer2 = codec[1](frame);
		const tag = deserializer2[5]();
		deserializer2[6](tag, 2);
		deserializer2[11]();
		const value2 = $as(deserializer2);
		const $cB = deserializer2[16]();
		let $cC = null;
		if ($cB[0] === 0) {
			const _reason = $cB[1];
			$cC = undefined;
		} else {
			$cC = $bt(cache, [ 0, value2 ], $bp);
		}
		return $cC;
	};
	self[2].v.push([ channel, deliver ]);
	return [ channel, encode_control(self[1], "Subscribe", channel), self[0], cache ];
}
function $cD(self, observer) {
	send(self[2], self[1]);
	return $bE(self[3], (value2) => {
		const $cE = value2;
		let $cF = null;
		if ($cE[0] === 0) {
			const present = $cE[1];
			$cF = observer(present);
		} else {
			$cF = undefined;
		}
		return $cF;
	});
}
async function $cH(transport, codec, method, args) {
	const reply_frame = await (call(transport, encode_request(codec, method, args)));
	const deserializer2 = codec[1](reply_frame);
	const tag = deserializer2[5]();
	const $cI = tag;
	let $cJ = null;
	if ($cI === "Success") {
		deserializer2[6]("Success", 1);
		const value2 = $ap(deserializer2);
		const $cK = deserializer2[16]();
		let $cL = null;
		if ($cK[0] === 1) {
			$cL = [ 0, value2 ];
		} else {
			const reason = $cK[1];
			$cL = [ 1, [ 1, reason ] ];
		}
		$cJ = $cL;
	} else if ($cI === "Failure") {
		deserializer2[6]("Failure", 1);
		const error = $av(deserializer2);
		deserializer2[7]();
		$cJ = [ 1, error ];
	} else {
		$cJ = [ 1, [ 1, "unrecognized reply envelope" ] ];
	}
	return $cJ;
}
async function $cG(self) {
	return await ($cH(self[0], self[1], "whoami", [  ]));
}
function $cS(deserializer2) {
	return bool_value4(deserializer2);
}
async function $cP(transport, codec, method, args) {
	const reply_frame = await (call(transport, encode_request(codec, method, args)));
	const deserializer2 = codec[1](reply_frame);
	const tag = deserializer2[5]();
	const $cQ = tag;
	let $cR = null;
	if ($cQ === "Success") {
		deserializer2[6]("Success", 1);
		const value2 = $cS(deserializer2);
		const $cT = deserializer2[16]();
		let $cU = null;
		if ($cT[0] === 1) {
			$cU = [ 0, value2 ];
		} else {
			const reason = $cT[1];
			$cU = [ 1, [ 1, reason ] ];
		}
		$cR = $cU;
	} else if ($cQ === "Failure") {
		deserializer2[6]("Failure", 1);
		const error = $av(deserializer2);
		deserializer2[7]();
		$cR = [ 1, error ];
	} else {
		$cR = [ 1, [ 1, "unrecognized reply envelope" ] ];
	}
	return $cR;
}
async function $cO(self, username, password) {
	return await ($cP(self[0], self[1], "login", [ (serializer2) => {
		return $o(username, serializer2);
	}, (serializer2) => {
		return $o(password, serializer2);
	} ]));
}
const reactive_sessions = __shared_new([  ]);
const next_channel = __shared_new(0);
const next_subscriber_id = __shared_new(0);
const turn_scope = null;
const draining_turns = __shared_new([  ]);
const owner_scope = null;
(async () => {
	const transport = local_rpc(into_protocol(accounts_dispatcher(), json_codec()), [ 1 ]);
	const client = [ transport, json_codec() ];
	show(await ($ak(client, 1)));
	show(await ($ak(client, 9)));
	await (show_raw(transport));
	await (reactive_demo([ 1 ]));
	await (session_demo([ 1 ]));
})();
