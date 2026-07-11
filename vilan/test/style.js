function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function __map_get(map, key) {
	return map.has(key) ? [ 0, __clone(map.get(key)) ] : [ 1 ];
}
function __map_keys(map) {
	return [ ...map.keys() ].map(__clone);
}
function __map_values(map) {
	return [ ...map.values() ].map(__clone);
}
function class_list(self) {
	let out = "";
	for (const entry of __map_values(self[0])) {
		const $a = entry;
		const class2 = $a[0];
		const _declaration = $a[1];
		if (out === "") {
			out = class2;
		} else {
			out = out + " " + class2;
		}
	}
	return out;
}
function add(self, b) {
	let rules = __clone(self[0]);
	for (const key of __map_keys(b[0])) {
		const $b = __map_get(b[0], key);
		let $c = null;
		if ($b[0] === 0) {
			const entry = $b[1];
			$c = rules.set(key, entry);
		} else {
			$c = undefined;
		}
		$c;
	}
	return [ rules ];
}
const card = [ new Map([ [ "::display", [ "sbiovxm", "display:flex" ] ], [ "::padding", [ "s1ufvr2", "padding:var(--space-4)" ] ], [ "::background-color", [ "siolu0w", "background-color:var(--gray-50)" ] ], [ ":hover:background-color", [ "s1c7l5ao", "background-color:var(--gray-100)" ] ] ]) ];
const active = [ new Map([ [ "::padding", [ "s1ufvsw", "padding:var(--space-6)" ] ] ]) ];
console.log(class_list(card));
console.log(class_list(add(card, active)));
const wide = [ new Map([ [ "::width", [ "s178hckh", "width:37px" ] ] ]) ];
console.log(class_list(wide));
