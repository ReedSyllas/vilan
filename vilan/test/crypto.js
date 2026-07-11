function __at(list, index) {
	if (index >= 0 && index < list.length) return list[index];
	throw "index out of bounds: the length is " + list.length + " but the index is " + index;
}
async function __hmac_sha512(key, data) {
	const imported = await crypto.subtle.importKey("raw", key, { name: "HMAC", hash: "SHA-512" }, false, [ "sign" ]);
	return new Uint8Array(await crypto.subtle.sign("HMAC", imported, data));
}
function __list_pop(list) {
	return list.length === 0 ? [ 1 ] : [ 0, list.pop() ];
}
async function __pbkdf2_sha512(password, salt, iterations, bits) {
	const imported = await crypto.subtle.importKey("raw", password, "PBKDF2", false, [ "deriveBits" ]);
	return new Uint8Array(await crypto.subtle.deriveBits({ name: "PBKDF2", salt, iterations, hash: "SHA-512" }, imported, bits));
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
	const $q = __list_pop(self[2].v);
	let $r = null;
	if ($q[0] === 0) {
		const saved = $q[1];
		$r = saved;
	} else {
		$r = false;
	}
	self[1].v = $r;
}
function result(self) {
	return self[0].v;
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
function str_value(self, value2) {
	value(self, JSON.stringify(value2));
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
	const $J = self[1].v;
	let $K = null;
	if ($J[0] === 0) {
		const _reason = $J[1];
		$K = false;
	} else {
		$K = true;
	}
	return $K;
}
function report(self, reason) {
	const $G = self[1].v;
	let $H = null;
	if ($G[0] === 0) {
		const _first = $G[1];
		$H = undefined;
	} else {
		self[1].v = [ 0, reason ];
		$H = undefined;
	}
	return $H;
}
function top(self) {
	let $M = null;
	if (!(ok(self)) || $L(self[0].v)) {
		$M = JSON.parse("null");
	} else {
		const values = self[0].v;
		$M = __at(values, values.length - 1);
	}
	return $M;
}
function take(self) {
	if (!(ok(self))) {
		return JSON.parse("null");
	}
	const $P = __list_pop(self[0].v);
	let $Q = null;
	if ($P[0] === 0) {
		const value2 = $P[1];
		$Q = value2;
	} else {
		report(self, "unexpected end of document");
		$Q = JSON.parse("null");
	}
	return $Q;
}
function begin_struct2(self) {

}
function field2(self, name) {
	const subject = top(self);
	let $N = null;
	if (ok(self)) {
		if (Object.hasOwn(subject, name)) {
			self[0].v.push(subject[name]);
		} else {
			report(self, "missing field \'" + name + "\'");
		}
		$N = undefined;
	}
	return $N;
}
function end_struct2(self) {
	take(self);
}
function str_value2(self) {
	const value2 = take(self);
	let $R = null;
	if (ok(self)) {
		$R = String(value2);
	} else {
		$R = "";
	}
	return $R;
}
function bool_value2(self) {
	const value2 = take(self);
	let $T = null;
	if (ok(self)) {
		$T = Boolean(value2);
	} else {
		$T = false;
	}
	return $T;
}
function opened_reader(text) {
	const $E = __try_parse_json(text);
	let $F = null;
	if ($E[0] === 0) {
		const root = $E[1];
		$F = new3(root);
	} else {
		const reader = new3(JSON.parse("null"));
		report(reader, "malformed JSON");
		$F = reader;
	}
	return $F;
}
function set(self, index, value2) {
	self.fill(value2, index, index + 1);
}
function encode_utf8(text) {
	return new TextEncoder().encode(text);
}
function decode_utf8(bytes) {
	return new TextDecoder().decode(bytes);
}
function new4(start, end) {
	return [ start, end ];
}
function next(self) {
	let $a = null;
	if (self[0] < self[1]) {
		const value2 = self[0];
		self[0] = self[0] + 1;
		$a = [ 0, value2 ];
	} else {
		$a = [ 1 ];
	}
	return $a;
}
function char_at(value2) {
	return alphabet.substring(value2, value2 + 1);
}
function encode_url(bytes) {
	const total = bytes.length;
	let out = "";
	const $b = new4(0, Math.trunc(total / 3));
	while (true) {
		const $c = next($b);
		if ($c[0] !== 0) {
			break;
		}
		const group = $c[1];
		const base = group * 3;
		const chunk = bytes.at(base) << 16 | bytes.at(base + 1) << 8 | bytes.at(base + 2);
		out = out + char_at(chunk >> 18 & 63) + char_at(chunk >> 12 & 63) + char_at(chunk >> 6 & 63) + char_at(chunk & 63);
	}
	const rest = total % 3;
	if (rest === 1) {
		const chunk2 = bytes.at(total - 1) << 16;
		out = out + char_at(chunk2 >> 18 & 63) + char_at(chunk2 >> 12 & 63);
	}
	if (rest === 2) {
		const chunk3 = bytes.at(total - 2) << 16 | bytes.at(total - 1) << 8;
		out = out + char_at(chunk3 >> 18 & 63) + char_at(chunk3 >> 12 & 63) + char_at(chunk3 >> 6 & 63);
	}
	return out;
}
function digit(code) {
	const c = as_i32(code);
	if (c >= 65 && c <= 90) {
		return c - 65;
	}
	if (c >= 97 && c <= 122) {
		return c - 71;
	}
	if (c >= 48 && c <= 57) {
		return c + 4;
	}
	if (c === 45) {
		return 62;
	}
	if (c === 95) {
		return 63;
	}
	return 0 - 1;
}
function decode_url(text) {
	const length = text.length;
	const rest = length % 4;
	if (rest === 1) {
		return [ 1 ];
	}
	const full = Math.trunc(length / 4);
	const $d = rest;
	let $e = null;
	if ($d === 2) {
		$e = 1;
	} else if ($d === 3) {
		$e = 2;
	} else {
		$e = 0;
	}
	const tail_bytes = $e;
	let out = new Uint8Array(full * 3 + tail_bytes);
	let write = 0;
	const $f = new4(0, full);
	while (true) {
		const $g = next($f);
		if ($g[0] !== 0) {
			break;
		}
		const group = $g[1];
		const base = group * 4;
		const a = digit(text.charCodeAt(base));
		const b = digit(text.charCodeAt(base + 1));
		const c = digit(text.charCodeAt(base + 2));
		const d = digit(text.charCodeAt(base + 3));
		if (a < 0 || b < 0 || c < 0 || d < 0) {
			return [ 1 ];
		}
		const chunk = a << 18 | b << 12 | c << 6 | d;
		set(out, write, chunk >> 16 & 255);
		set(out, write + 1, chunk >> 8 & 255);
		set(out, write + 2, chunk & 255);
		write = write + 3;
	}
	if (rest === 2) {
		const a2 = digit(text.charCodeAt(length - 2));
		const b2 = digit(text.charCodeAt(length - 1));
		if (a2 < 0 || b2 < 0) {
			return [ 1 ];
		}
		set(out, write, (a2 << 6 | b2) >> 4 & 255);
	}
	if (rest === 3) {
		const a3 = digit(text.charCodeAt(length - 3));
		const b3 = digit(text.charCodeAt(length - 2));
		const c2 = digit(text.charCodeAt(length - 1));
		if (a3 < 0 || b3 < 0 || c2 < 0) {
			return [ 1 ];
		}
		const chunk2 = a3 << 12 | b3 << 6 | c2;
		set(out, write, chunk2 >> 10 & 255);
		set(out, write + 1, chunk2 >> 2 & 255);
	}
	return [ 0, out ];
}
function equals_constant_time(a, b) {
	if (a.length !== b.length) {
		return false;
	}
	let acc = 0;
	const $v = new4(0, a.length);
	while (true) {
		const $w = next($v);
		if ($w[0] !== 0) {
			break;
		}
		const index = $w[1];
		acc = acc | a.at(index) ^ b.at(index);
	}
	return acc === 0;
}
async function verified_segment(secret, token) {
	const parts = token.split(".");
	const shaped = parts.length === 3 && __at(parts, 0) === header_segment;
	const expected = await (__hmac_sha512(secret, encode_utf8(__at(parts, 0) + "." + __at(parts, 1))));
	const $t = decode_url(__at(parts, 2));
	let $u = null;
	if ($t[0] === 0) {
		const given = $t[1];
		let $x = null;
		if (shaped && equals_constant_time(expected, given)) {
			$x = [ 0, __at(parts, 1) ];
		} else {
			$x = [ 1 ];
		}
		$u = $x;
	} else {
		$u = [ 1 ];
	}
	return $u;
}
function fold_unsigned(value2, modulus) {
	const truncated = Math.trunc(value2);
	const wrapped = truncated % modulus;
	let $h = null;
	if (wrapped < 0) {
		$h = wrapped + modulus;
	} else {
		$h = wrapped;
	}
	return $h;
}
function fold_signed(value2, modulus, half) {
	const wrapped = fold_unsigned(value2, modulus);
	let $i = null;
	if (wrapped >= half) {
		$i = wrapped - modulus;
	} else {
		$i = wrapped;
	}
	return $i;
}
function as_i32(self) {
	const widened = Number(self);
	return Number(fold_signed(widened, 4294967296, 2147483648));
}
function $o(self, serializer) {
	str_value(serializer, self);
}
function $p(self, serializer) {
	bool_value(serializer, self);
}
function $n(self, serializer) {
	begin_struct(serializer, 2);
	field(serializer, "user");
	$o(self[0], serializer);
	field(serializer, "admin");
	$p(self[1], serializer);
	end_struct(serializer);
}
function $m(value2) {
	const writer = new2();
	$n(value2, writer);
	return result(writer);
}
async function $l(secret, claims) {
	const payload = encode_url(encode_utf8($m(claims)));
	const signing_input = header_segment + "." + payload;
	const signature = await (__hmac_sha512(secret, encode_utf8(signing_input)));
	return signing_input + "." + encode_url(signature);
}
function $L(self) {
	return self.length === 0;
}
function $O(deserializer) {
	return str_value2(deserializer);
}
function $S(deserializer) {
	return bool_value2(deserializer);
}
function $I(deserializer) {
	begin_struct2(deserializer);
	field2(deserializer, "user");
	const user = $O(deserializer);
	field2(deserializer, "admin");
	const admin = $S(deserializer);
	end_struct2(deserializer);
	return [ user, admin ];
}
function $D(text) {
	const reader = opened_reader(text);
	const value2 = $I(reader);
	const $U = reader[1].v;
	let $V = null;
	if ($U[0] === 1) {
		$V = [ 0, value2 ];
	} else {
		const reason = $U[1];
		$V = [ 1, reason ];
	}
	return $V;
}
function $A(segment) {
	const $B = decode_url(segment);
	let $C = null;
	if ($B[0] === 0) {
		const payload = $B[1];
		const decoded = $D(decode_utf8(payload));
		const $W = decoded;
		let $X = null;
		if ($W[0] === 0) {
			const claims = $W[1];
			$X = [ 0, claims ];
		} else {
			const _reason = $W[1];
			$X = [ 1 ];
		}
		$C = $X;
	} else {
		$C = [ 1 ];
	}
	return $C;
}
async function $s(secret, token) {
	const segment = await (verified_segment(secret, token));
	const $y = segment;
	let $z = null;
	if ($y[0] === 0) {
		const payload = $y[1];
		$z = $A(payload);
	} else {
		$z = [ 1 ];
	}
	return $z;
}
const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const header_segment = "eyJhbGciOiJIUzUxMiIsInR5cCI6IkpXVCJ9";
(async () => {
	const $j = decode_url(encode_url(encode_utf8("payload")));
	let $k = null;
	if ($j[0] === 0) {
		const bytes = $j[1];
		$k = console.log(decode_utf8(bytes));
	} else {
		$k = console.log("decode failed");
	}
	$k;
	const secret = encode_utf8("server-signing-key");
	const token = await ($l(secret, [ "reed", true ]));
	const verified = await ($s(secret, token));
	const $Y = verified;
	let $Z = null;
	if ($Y[0] === 0) {
		const session = $Y[1];
		$Z = console.log("welcome " + session[0] + " (admin=" + session[1] + ")");
	} else {
		$Z = console.log("unauthorized");
	}
	$Z;
	const forged = await ($s(encode_utf8("attacker-key"), token));
	const $aa = forged;
	let $ab = null;
	if ($aa[0] === 0) {
		const _s = $aa[1];
		$ab = console.log("SECURITY BUG");
	} else {
		$ab = console.log("forged token rejected");
	}
	$ab;
	const salt = encode_utf8("per-user-salt");
	const first = await (__pbkdf2_sha512(encode_utf8("hunter2"), salt, 1000, 512));
	const again = await (__pbkdf2_sha512(encode_utf8("hunter2"), salt, 1000, 512));
	console.log(equals_constant_time(first, again));
})();
