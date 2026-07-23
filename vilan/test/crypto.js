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
function set(self, index, value2) {
	self.fill(value2, index, index + 1);
}
function encode_utf8(text) {
	return new TextEncoder().encode(text);
}
function decode_utf8(bytes) {
	return new TextDecoder().decode(bytes);
}
function equals_constant_time(a, b) {
	if (a.length !== b.length) {
		return false;
	}
	let acc = 0;
	const $w = new4(0, a.length);
	while (true) {
		const $x = next($w);
		if ($x[0] !== 0) {
			break;
		}
		const index = $x[1];
		acc = acc | a.at(index) ^ b.at(index);
	}
	return acc === 0;
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
	const $H = self[1].v;
	let $I = null;
	if ($H[0] === 0) {
		const _reason = $H[1];
		$I = false;
	} else {
		$I = true;
	}
	return $I;
}
function report(self, reason) {
	const $E = self[1].v;
	let $F = null;
	if ($E[0] === 0) {
		const _first = $E[1];
		$F = undefined;
	} else {
		self[1].v = [ 0, reason ];
		$F = undefined;
	}
	return $F;
}
function top(self) {
	let $K = null;
	if (!(ok(self)) || $J(self[0].v)) {
		$K = JSON.parse("null");
	} else {
		const values = self[0].v;
		$K = __at(values, values.length - 1);
	}
	return $K;
}
function take(self) {
	if (!(ok(self))) {
		return JSON.parse("null");
	}
	const $N = __list_pop(self[0].v);
	let $O = null;
	if ($N[0] === 0) {
		const value2 = $N[1];
		$O = value2;
	} else {
		report(self, "unexpected end of document");
		$O = JSON.parse("null");
	}
	return $O;
}
function begin_struct2(self) {

}
function field2(self, name) {
	const subject = top(self);
	let $L = null;
	if (ok(self)) {
		if (Object.hasOwn(subject, name)) {
			self[0].v.push(subject[name]);
		} else {
			report(self, "missing field \'" + name + "\'");
		}
		$L = undefined;
	}
	return $L;
}
function end_struct2(self) {
	take(self);
}
function str_value2(self) {
	const value2 = take(self);
	let $P = null;
	if (ok(self)) {
		$P = String(value2);
	} else {
		$P = "";
	}
	return $P;
}
function bool_value2(self) {
	const value2 = take(self);
	let $R = null;
	if (ok(self)) {
		$R = Boolean(value2);
	} else {
		$R = false;
	}
	return $R;
}
function opened_reader(text) {
	const $C = __try_parse_json(text);
	let $D = null;
	if ($C[0] === 0) {
		const root = $C[1];
		$D = new3(root);
	} else {
		const reader = new3(JSON.parse("null"));
		report(reader, "malformed JSON");
		$D = reader;
	}
	return $D;
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
function $J(self) {
	return self.length === 0;
}
function $M(deserializer) {
	return str_value2(deserializer);
}
function $Q(deserializer) {
	return bool_value2(deserializer);
}
function $G(deserializer) {
	begin_struct2(deserializer);
	field2(deserializer, "user");
	const user = $M(deserializer);
	field2(deserializer, "admin");
	const admin = $Q(deserializer);
	end_struct2(deserializer);
	return [ user, admin ];
}
function $B(text) {
	const reader = opened_reader(text);
	const value2 = $G(reader);
	const $S = reader[1].v;
	let $T = null;
	if ($S[0] === 1) {
		$T = [ 0, value2 ];
	} else {
		const reason = $S[1];
		$T = [ 1, reason ];
	}
	return $T;
}
function $y(segment) {
	const $z = decode_url(segment);
	let $A = null;
	if ($z[0] === 0) {
		const payload = $z[1];
		const decoded = $B(decode_utf8(payload));
		const $U = decoded;
		let $V = null;
		if ($U[0] === 0) {
			const claims = $U[1];
			$V = [ 0, claims ];
		} else {
			const _reason = $U[1];
			$V = [ 1 ];
		}
		$A = $V;
	} else {
		$A = [ 1 ];
	}
	return $A;
}
async function $s(secret, token) {
	const parts = token.split(".");
	let $t = null;
	if (parts.length !== 3 || __at(parts, 0) !== header_segment) {
		$t = [ 1 ];
	} else {
		const expected = await (__hmac_sha512(secret, encode_utf8(__at(parts, 0) + "." + __at(parts, 1))));
		const $u = decode_url(__at(parts, 2));
		let $v = null;
		if ($u[0] === 0) {
			const given = $u[1];
			let $W = null;
			if (equals_constant_time(expected, given)) {
				$W = $y(__at(parts, 1));
			} else {
				$W = [ 1 ];
			}
			$v = $W;
		} else {
			$v = [ 1 ];
		}
		$t = $v;
	}
	return $t;
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
	const $X = verified;
	let $Y = null;
	if ($X[0] === 0) {
		const session = $X[1];
		$Y = console.log("welcome " + session[0] + " (admin=" + session[1] + ")");
	} else {
		$Y = console.log("unauthorized");
	}
	$Y;
	const forged = await ($s(encode_utf8("attacker-key"), token));
	const $Z = forged;
	let $aa = null;
	if ($Z[0] === 0) {
		const _s = $Z[1];
		$aa = console.log("SECURITY BUG");
	} else {
		$aa = console.log("forged token rejected");
	}
	$aa;
	const salt = encode_utf8("per-user-salt");
	const first = await (__pbkdf2_sha512(encode_utf8("hunter2"), salt, 1000, 512));
	const again = await (__pbkdf2_sha512(encode_utf8("hunter2"), salt, 1000, 512));
	console.log(equals_constant_time(first, again));
})();
