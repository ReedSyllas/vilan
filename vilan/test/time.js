function __at(list, index) {
	if (index >= 0 && index < list.length) return list[index];
	throw "index out of bounds: the length is " + list.length + " but the index is " + index;
}
function __json_tag(value) {
	return typeof value === "string" ? value : Object.keys(value)[0];
}
function __list_pop(list) {
	return list.length === 0 ? [ 1 ] : [ 0, list.pop() ];
}
function __shared_new(value) {
	return { v: value };
}
function __sleep(ms, signal) {
	const sig = signal && signal[0] === 0 ? signal[1] : undefined;
	return new Promise((resolve, reject) => {
		if (sig && sig.aborted) {
			reject(sig.reason);
			return;
		}
		const timer = setTimeout(() => resolve(), ms);
		if (sig) sig.addEventListener("abort", () => {
			clearTimeout(timer);
			reject(sig.reason);
		}, { once: true });
	});
}
function __try_parse_json(text) {
	try {
		return [ 0, JSON.parse(text) ];
	} catch (error) {
		return [ 1 ];
	}
}
function new2() {
	return [ __shared_new(new Uint8Array(64)), __shared_new(0) ];
}
function ensure(self, extra) {
	const needed = self[1].v + extra;
	let capacity = self[0].v.length;
	if (needed > capacity) {
		while (needed > capacity) {
			capacity = capacity * 2;
		}
		const grown = new Uint8Array(capacity);
		grown.set(self[0].v, 0);
		self[0].v = grown;
	}
}
function write_byte(self, value2) {
	ensure(self, 1);
	set(self[0].v, self[1].v, value2);
	self[1].v = self[1].v + 1;
}
function write_i32(self, value2) {
	write_byte(self, value2);
	write_byte(self, value2 >> 8);
	write_byte(self, value2 >> 16);
	write_byte(self, value2 >> 24);
}
function write_byte_u32(self, value2) {
	ensure(self, 1);
	set_u32(self[0].v, self[1].v, value2);
	self[1].v = self[1].v + 1;
}
function write_u32(self, value2) {
	write_byte_u32(self, (value2 & 0xFF) >>> 0);
	write_byte_u32(self, (value2 >>> 8 & 0xFF) >>> 0);
	write_byte_u32(self, (value2 >>> 16 & 0xFF) >>> 0);
	write_byte_u32(self, value2 >>> 24);
}
function write_f64(self, value2) {
	const scratch = new Uint8Array(8);
	new DataView(scratch.buffer).setFloat64(0, value2, true);
	ensure(self, 8);
	self[0].v.set(scratch, self[1].v);
	self[1].v = self[1].v + 8;
}
function write_str(self, value2) {
	const encoded = encode_utf8(value2);
	write_i32(self, encoded.length);
	ensure(self, encoded.length);
	self[0].v.set(encoded, self[1].v);
	self[1].v = self[1].v + encoded.length;
}
function finish(self) {
	return self[0].v.slice(0, self[1].v);
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
		return i53_value(self, value2);
	}, (value2) => {
		return f64_value(self, value2);
	}, (value2) => {
		return bool_value(self, value2);
	} ];
}
function begin_struct(self, fields) {

}
function field(self, name) {

}
function end_struct(self) {

}
function begin_list(self, length) {
	write_i32(self, length);
}
function end_list(self) {

}
function begin_variant(self, name, arity) {
	write_str(self, name);
}
function end_variant(self) {

}
function null_value(self) {
	write_byte(self, 0);
}
function some_value(self) {
	write_byte(self, 1);
}
function str_value(self, value2) {
	write_str(self, value2);
}
function i32_value(self, value2) {
	write_i32(self, value2);
}
function u32_value(self, value2) {
	write_u32(self, value2);
}
function i53_value(self, value2) {
	write_f64(self, Number(value2));
}
function f64_value(self, value2) {
	write_f64(self, value2);
}
function bool_value(self, value2) {
	let $ac = null;
	if (value2) {
		$ac = 1;
	} else {
		$ac = 0;
	}
	write_byte(self, $ac);
}
function new3(bytes) {
	return [ bytes, __shared_new(0), __shared_new([ 1 ]) ];
}
function ok(self) {
	const $af = self[2].v;
	let $ag = null;
	if ($af[0] === 0) {
		const _reason = $af[1];
		$ag = false;
	} else {
		$ag = true;
	}
	return $ag;
}
function report(self, reason) {
	const $ah = self[2].v;
	let $ai = null;
	if ($ah[0] === 0) {
		const _first = $ah[1];
		$ai = undefined;
	} else {
		self[2].v = [ 0, reason ];
		$ai = undefined;
	}
	return $ai;
}
function expect(self, count) {
	if (!(ok(self))) {
		return false;
	}
	if (self[1].v + count > self[0].length) {
		report(self, "unexpected end of frame");
		return false;
	}
	return true;
}
function read_byte(self) {
	if (!(expect(self, 1))) {
		return 0;
	}
	const value2 = self[0].at(self[1].v);
	self[1].v = self[1].v + 1;
	return value2;
}
function read_i32(self) {
	if (!(expect(self, 4))) {
		return 0;
	}
	const at = self[1].v;
	const value2 = self[0].at(at) | self[0].at(at + 1) << 8 | self[0].at(at + 2) << 16 | self[0].at(at + 3) << 24;
	self[1].v = at + 4;
	return value2;
}
function read_u32(self) {
	if (!(expect(self, 4))) {
		return 0;
	}
	const at = self[1].v;
	const value2 = (((self[0].at(at) | self[0].at(at + 1) << 8 >>> 0) >>> 0 | self[0].at(at + 2) << 16 >>> 0) >>> 0 | self[0].at(at + 3) << 24 >>> 0) >>> 0;
	self[1].v = at + 4;
	return value2;
}
function read_f64(self) {
	if (!(expect(self, 8))) {
		return 0.0;
	}
	const at = self[1].v;
	const scratch = self[0].slice(at, at + 8);
	self[1].v = at + 8;
	return new DataView(scratch.buffer).getFloat64(0, true);
}
function read_length(self) {
	const length = read_i32(self);
	if (length < 0 || self[1].v + length > self[0].length) {
		report(self, "length prefix exceeds frame");
		return 0;
	}
	return length;
}
function read_str(self) {
	const length = read_length(self);
	if (!(expect(self, length))) {
		return "";
	}
	const at = self[1].v;
	const piece = self[0].slice(at, at + length);
	self[1].v = at + length;
	return decode_utf8(piece);
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
		return i53_value2(self);
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

}
function end_struct2(self) {

}
function begin_list2(self) {
	return read_length(self);
}
function end_list2(self) {

}
function variant_tag(self) {
	return read_str(self);
}
function begin_variant2(self, name, arity) {

}
function end_variant2(self) {

}
function is_null(self) {
	if (!(expect(self, 1))) {
		return false;
	}
	let $aj = null;
	if (self[0].at(self[1].v) === 0) {
		$aj = true;
	} else {
		self[1].v = self[1].v + 1;
		$aj = false;
	}
	return $aj;
}
function null_value2(self) {
	read_byte(self);
}
function str_value2(self) {
	return read_str(self);
}
function i32_value2(self) {
	return read_i32(self);
}
function u32_value2(self) {
	return read_u32(self);
}
function i53_value2(self) {
	return as_i53(read_f64(self));
}
function f64_value2(self) {
	return read_f64(self);
}
function bool_value2(self) {
	return read_byte(self) !== 0;
}
function fail(self, reason) {
	report(self, reason);
}
function failed(self) {
	return self[2].v;
}
function binary_codec() {
	return [ () => {
		const writer = new2();
		return [ serializer(writer), () => {
			return [ 1, finish(writer) ];
		} ];
	}, (frame) => {
		const $ad = frame;
		let $ae = null;
		if ($ad[0] === 1) {
			const bytes = $ad[1];
			$ae = deserializer(new3(bytes));
		} else {
			const text = $ad[1];
			const reader = new3(new Uint8Array(0));
			report(reader, "binary codec: received a text frame");
			$ae = deserializer(reader);
		}
		return $ae;
	} ];
}
function set(self, index, value2) {
	self.fill(value2, index, index + 1);
}
function set_u32(self, index, value2) {
	self.fill(value2, index, index + 1);
}
function encode_utf8(text) {
	return new TextEncoder().encode(text);
}
function decode_utf8(bytes) {
	return new TextDecoder().decode(bytes);
}
function new4() {
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
	const $m = __list_pop(self[2].v);
	let $n = null;
	if ($m[0] === 0) {
		const saved = $m[1];
		$n = saved;
	} else {
		$n = false;
	}
	self[1].v = $n;
}
function result(self) {
	return self[0].v;
}
function serializer2(self) {
	return [ (fields) => {
		return begin_struct3(self, fields);
	}, (name) => {
		return field3(self, name);
	}, () => {
		return end_struct3(self);
	}, (length) => {
		return begin_list3(self, length);
	}, () => {
		return end_list3(self);
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
		return u32_value3(self, value2);
	}, (value2) => {
		return i53_value3(self, value2);
	}, (value2) => {
		return f64_value3(self, value2);
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
function begin_list3(self, length) {
	open(self, "[");
}
function end_list3(self) {
	close(self, "]");
}
function begin_variant3(self, name, arity) {
	self[3].v.push(arity);
	let $o = null;
	if (arity === 0) {
		value(self, JSON.stringify(name));
	} else {
		open(self, "{");
		self[0].v = self[0].v + JSON.stringify(name) + ":";
		self[1].v = false;
		if (arity > 1) {
			self[0].v = self[0].v + "[";
		}
		$o = undefined;
	}
	return $o;
}
function end_variant3(self) {
	const $p = __list_pop(self[3].v);
	let $q = null;
	if ($p[0] === 0) {
		const opened = $p[1];
		$q = opened;
	} else {
		$q = 0;
	}
	const arity = $q;
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
function u32_value3(self, value2) {
	value(self, "" + value2);
}
function i53_value3(self, value2) {
	value(self, "" + value2);
}
function f64_value3(self, value2) {
	value(self, "" + value2);
}
function bool_value3(self, value2) {
	value(self, "" + value2);
}
function new5(root) {
	const stack = __shared_new([  ]);
	stack.v.push(root);
	return [ stack, __shared_new([ 1 ]) ];
}
function ok2(self) {
	const $x = self[1].v;
	let $y = null;
	if ($x[0] === 0) {
		const _reason = $x[1];
		$y = false;
	} else {
		$y = true;
	}
	return $y;
}
function report2(self, reason) {
	const $v = self[1].v;
	let $w = null;
	if ($v[0] === 0) {
		const _first = $v[1];
		$w = undefined;
	} else {
		self[1].v = [ 0, reason ];
		$w = undefined;
	}
	return $w;
}
function top(self) {
	let $A = null;
	if (!(ok2(self)) || $z(self[0].v)) {
		$A = JSON.parse("null");
	} else {
		const values = self[0].v;
		$A = __at(values, values.length - 1);
	}
	return $A;
}
function take(self) {
	if (!(ok2(self))) {
		return JSON.parse("null");
	}
	const $C = __list_pop(self[0].v);
	let $D = null;
	if ($C[0] === 0) {
		const value2 = $C[1];
		$D = value2;
	} else {
		report2(self, "unexpected end of document");
		$D = JSON.parse("null");
	}
	return $D;
}
function deserializer2(self) {
	return [ () => {
		return begin_struct4(self);
	}, (name) => {
		return field4(self, name);
	}, () => {
		return end_struct4(self);
	}, () => {
		return begin_list4(self);
	}, () => {
		return end_list4(self);
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
		return u32_value4(self);
	}, () => {
		return i53_value4(self);
	}, () => {
		return f64_value4(self);
	}, () => {
		return bool_value4(self);
	}, (reason) => {
		return fail2(self, reason);
	}, () => {
		return failed2(self);
	} ];
}
function begin_struct4(self) {

}
function field4(self, name) {
	const subject = top(self);
	let $B = null;
	if (ok2(self)) {
		if (Object.hasOwn(subject, name)) {
			self[0].v.push(subject[name]);
		} else {
			report2(self, "missing field \'" + name + "\'");
		}
		$B = undefined;
	}
	return $B;
}
function end_struct4(self) {
	take(self);
}
function begin_list4(self) {
	const subject = take(self);
	let $E = null;
	if (ok2(self)) {
		const elements = subject;
		let index = elements.length - 1;
		while (index >= 0) {
			self[0].v.push(__at(elements, index));
			index = index - 1;
		}
		$E = elements.length;
	} else {
		$E = 0;
	}
	return $E;
}
function end_list4(self) {

}
function variant_tag2(self) {
	let $F = null;
	if (ok2(self)) {
		$F = __json_tag(top(self));
	} else {
		$F = "";
	}
	return $F;
}
function begin_variant4(self, name, arity) {
	const subject = take(self);
	let $I = null;
	if (ok2(self) && arity > 0) {
		let $H = null;
		if (Object.hasOwn(subject, name)) {
			const payload = subject[name];
			let $G = null;
			if (arity === 1) {
				self[0].v.push(payload);
			} else {
				const elements = payload;
				let index = elements.length - 1;
				while (index >= 0) {
					self[0].v.push(__at(elements, index));
					index = index - 1;
				}
				$G = undefined;
			}
			$H = $G;
		} else {
			report2(self, "missing payload for variant \'" + name + "\'");
		}
		$I = $H;
	}
	return $I;
}
function end_variant4(self) {

}
function is_null2(self) {
	return ok2(self) && top(self) === null;
}
function null_value4(self) {
	take(self);
}
function str_value4(self) {
	const value2 = take(self);
	let $J = null;
	if (ok2(self)) {
		$J = String(value2);
	} else {
		$J = "";
	}
	return $J;
}
function i32_value4(self) {
	const value2 = take(self);
	let $K = null;
	if (ok2(self)) {
		$K = Number(value2);
	} else {
		$K = 0;
	}
	return $K;
}
function u32_value4(self) {
	const value2 = take(self);
	let $L = null;
	if (ok2(self)) {
		$L = Number(value2);
	} else {
		$L = 0;
	}
	return $L;
}
function i53_value4(self) {
	const value2 = take(self);
	let $M = null;
	if (ok2(self)) {
		$M = Number(value2);
	} else {
		$M = 0;
	}
	return $M;
}
function f64_value4(self) {
	const value2 = take(self);
	let $N = null;
	if (ok2(self)) {
		$N = Number(value2);
	} else {
		$N = 0.0;
	}
	return $N;
}
function bool_value4(self) {
	const value2 = take(self);
	let $O = null;
	if (ok2(self)) {
		$O = Boolean(value2);
	} else {
		$O = false;
	}
	return $O;
}
function fail2(self, reason) {
	report2(self, reason);
}
function failed2(self) {
	return self[1].v;
}
function opened_reader(text) {
	const $t = __try_parse_json(text);
	let $u = null;
	if ($t[0] === 0) {
		const root = $t[1];
		$u = new5(root);
	} else {
		const reader = new5(JSON.parse("null"));
		report2(reader, "malformed JSON");
		$u = reader;
	}
	return $u;
}
function json_codec() {
	return [ () => {
		const writer = new4();
		return [ serializer2(writer), () => {
			return [ 0, result(writer) ];
		} ];
	}, (frame) => {
		const $r = frame;
		let $s = null;
		if ($r[0] === 0) {
			const text = $r[1];
			$s = deserializer2(opened_reader(text));
		} else {
			const bytes = $r[1];
			$s = deserializer2(opened_reader(decode_utf8(bytes)));
		}
		return $s;
	} ];
}
function partial_compare(self, b) {
	let $b = null;
	if (self < b) {
		$b = [ 0, -1 ];
	} else {
		let $c = null;
		if (self > b) {
			$c = [ 0, 1 ];
		} else {
			$c = [ 0, 0 ];
		}
		$b = $c;
	}
	return $b;
}
function fold_unsigned(value2, modulus) {
	const truncated = Math.trunc(value2);
	const wrapped = truncated % modulus;
	let $aw = null;
	if (wrapped < 0) {
		$aw = wrapped + modulus;
	} else {
		$aw = wrapped;
	}
	return $aw;
}
function fold_signed(value2, modulus, half) {
	const wrapped = fold_unsigned(value2, modulus);
	let $ax = null;
	if (wrapped >= half) {
		$ax = wrapped - modulus;
	} else {
		$ax = wrapped;
	}
	return $ax;
}
function as_i32(self) {
	const widened = Number(self);
	return Number(fold_signed(widened, 4294967296, 2147483648));
}
function as_i53(self) {
	const widened = self;
	return Number(Math.trunc(widened));
}
function now() {
	return [ as_i53(Date.now()) ];
}
function since(self, earlier) {
	return [ self[0] - earlier[0] ];
}
function to_iso(self) {
	return new Date(self[0]).toISOString();
}
function add(self, b) {
	return [ self[0] + b[0] ];
}
function sub(self, b) {
	return [ self[0] - b[0] ];
}
function partial_compare2(self, b) {
	return partial_compare(self[0], b[0]);
}
function millis(count) {
	return [ count ];
}
function seconds(count) {
	return [ count * 1000 ];
}
function minutes(count) {
	return [ count * 60000 ];
}
function hours(count) {
	return [ count * 3600000 ];
}
function days(count) {
	return [ count * 86400000 ];
}
function as_minutes(self) {
	return Math.trunc(self[0] / 60000);
}
function as_hours(self) {
	return Math.trunc(self[0] / 3600000);
}
function as_days(self) {
	return Math.trunc(self[0] / 86400000);
}
function describe(self) {
	let $g = null;
	if (self[0] < 0) {
		$g = "-";
	} else {
		$g = "";
	}
	const sign = $g;
	let $h = null;
	if (self[0] < 0) {
		$h = 0 - self[0];
	} else {
		$h = self[0];
	}
	const magnitude = $h;
	const parts = split_units(magnitude);
	return sign + parts;
}
function split_units(millis2) {
	const days2 = Math.trunc(millis2 / 86400000);
	const hours2 = Math.trunc(millis2 / 3600000) % 24;
	const minutes2 = Math.trunc(millis2 / 60000) % 60;
	const seconds2 = Math.trunc(millis2 / 1000) % 60;
	let $j = null;
	if (days2 > 0) {
		$j = join_units("" + days2 + "d", hours2, "h");
	} else if (hours2 > 0) {
		$j = join_units("" + hours2 + "h", minutes2, "m");
	} else if (minutes2 > 0) {
		$j = join_units("" + minutes2 + "m", seconds2, "s");
	} else if (seconds2 > 0) {
		$j = "" + seconds2 + "s";
	} else {
		$j = "" + millis2 + "ms";
	}
	return $j;
}
function join_units(head, count, unit) {
	let $i = null;
	if (count > 0) {
		$i = "" + head + " " + count + unit;
	} else {
		$i = head;
	}
	return $i;
}
function add2(self, b) {
	return [ self[0] + b[0] ];
}
function sub2(self, b) {
	return [ self[0] - b[0] ];
}
function partial_compare3(self, b) {
	return partial_compare(self[0], b[0]);
}
async function sleep(ms, $ay) {
	await (__sleep(ms, ambient_signal($ay)));
}
async function sleep_for(duration, $av) {
	await (sleep(as_i32(duration[0]), $av));
}
function eq(self, other) {
	return self[0] === other[0];
}
function ambient_signal($az) {
	const $aA = $az;
	let $aB = null;
	if ($aA[0] === 0) {
		const n = $aA[1];
		$aB = [ 0, n.signal_of() ];
	} else {
		$aB = [ 1 ];
	}
	return $aB;
}
function begin_struct5(self, fields) {
	self[0](fields);
}
function field5(self, name) {
	self[1](name);
}
function end_struct5(self) {
	self[2]();
}
function str_value5(self, value2) {
	self[9](value2);
}
function i53_value5(self, value2) {
	self[12](value2);
}
function begin_struct6(self) {
	self[0]();
}
function field6(self, name) {
	self[1](name);
}
function end_struct6(self) {
	self[2]();
}
function str_value6(self) {
	return self[10]();
}
function i53_value6(self) {
	return self[13]();
}
function eq2(self, other) {
	return self[0] === other[0] && self[1] === other[1];
}
function $a(self, b) {
	const $d = partial_compare2(self, b);
	return $d[0] === 0 && $d[1] > 0;
}
function $e(self, b) {
	const $f = partial_compare2(self, b);
	return $f[0] === 0 && $f[1] < 0;
}
function $k(self, b) {
	const $l = partial_compare3(self, b);
	return $l[0] === 0 && $l[1] > 0;
}
function $z(self) {
	return self.length === 0;
}
function $S(self, serializer3) {
	i53_value5(serializer3, self);
}
function $T(self, serializer3) {
	str_value5(serializer3, self);
}
function $R(self, serializer3) {
	begin_struct5(serializer3, 2);
	field5(serializer3, "at");
	$S(self[0], serializer3);
	field5(serializer3, "label");
	$T(self[1], serializer3);
	end_struct5(serializer3);
}
function $P(codec, value2) {
	const $Q = codec[0]();
	const serializer3 = $Q[0];
	const finish2 = $Q[1];
	$R(value2, serializer3);
	return finish2();
}
function $W(deserializer3) {
	return i53_value6(deserializer3);
}
function $X(deserializer3) {
	return str_value6(deserializer3);
}
function $V(deserializer3) {
	begin_struct6(deserializer3);
	field6(deserializer3, "at");
	const at = $W(deserializer3);
	field6(deserializer3, "label");
	const label = $X(deserializer3);
	end_struct6(deserializer3);
	return [ at, label ];
}
function $U(codec, frame) {
	const deserializer3 = codec[1](frame);
	const value2 = $V(deserializer3);
	const $Y = deserializer3[17]();
	let $Z = null;
	if ($Y[0] === 1) {
		$Z = [ 0, value2 ];
	} else {
		const reason = $Y[1];
		$Z = [ 1, reason ];
	}
	return $Z;
}
function $ao(self, serializer3) {
	begin_struct5(serializer3, 1);
	field5(serializer3, "millis");
	$S(self[0], serializer3);
	end_struct5(serializer3);
}
function $am(codec, value2) {
	const $an = codec[0]();
	const serializer3 = $an[0];
	const finish2 = $an[1];
	$ao(value2, serializer3);
	return finish2();
}
function $aq(deserializer3) {
	begin_struct6(deserializer3);
	field6(deserializer3, "millis");
	const millis2 = $W(deserializer3);
	end_struct6(deserializer3);
	return [ millis2 ];
}
function $ap(codec, frame) {
	const deserializer3 = codec[1](frame);
	const value2 = $aq(deserializer3);
	const $ar = deserializer3[17]();
	let $as = null;
	if ($ar[0] === 1) {
		$as = [ 0, value2 ];
	} else {
		const reason = $ar[1];
		$as = [ 1, reason ];
	}
	return $as;
}
(async () => {
	const epoch = [ 0 ];
	console.log(to_iso(epoch));
	const later = add(add(epoch, hours(2)), minutes(5));
	console.log(as_minutes(since(later, epoch)));
	console.log(as_hours(since(sub(later, minutes(5)), epoch)));
	console.log($a(later, epoch));
	console.log($e(epoch, later));
	console.log(eq(sub(later, minutes(125)), epoch));
	const span = add2(add2(days(1), hours(4)), seconds(30));
	console.log(as_hours(span));
	console.log(describe(span));
	console.log(describe(minutes(5)));
	console.log(describe(add2(seconds(90), millis(500))));
	console.log(describe(millis(980)));
	console.log(describe(millis(0)));
	console.log(describe(sub2(seconds(0), hours(3))));
	console.log(as_minutes(seconds(59)));
	console.log($k(hours(3), minutes(179)));
	console.log(as_days(since(now(), epoch)) > 19000);
	const stamp = [ 1720656000000, "k5" ];
	const json_back = $U(json_codec(), $P(json_codec(), stamp));
	const $aa = json_back;
	let $ab = null;
	if ($aa[0] === 0) {
		const value2 = $aa[1];
		$ab = console.log(eq2(value2, stamp));
	} else {
		const reason = $aa[1];
		$ab = console.log(reason);
	}
	$ab;
	const binary_back = $U(binary_codec(), $P(binary_codec(), stamp));
	const $ak = binary_back;
	let $al = null;
	if ($ak[0] === 0) {
		const value3 = $ak[1];
		$al = console.log("" + (value3[0] === stamp[0]) + " " + value3[1]);
	} else {
		const reason2 = $ak[1];
		$al = console.log(reason2);
	}
	$al;
	const sent = $ap(json_codec(), $am(json_codec(), later));
	const $at = sent;
	let $au = null;
	if ($at[0] === 0) {
		const value4 = $at[1];
		$au = console.log(eq(value4, later));
	} else {
		const reason3 = $at[1];
		$au = console.log(reason3);
	}
	$au;
	await (sleep_for(millis(10), [ 1 ]));
	console.log("slept");
})();
