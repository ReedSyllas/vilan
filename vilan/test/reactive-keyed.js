function __at(list, index) {
	if (index >= 0 && index < list.length) return list[index];
	throw "index out of bounds: the length is " + list.length + " but the index is " + index;
}
function __at_put(list, index, value) {
	if (index >= 0 && index < list.length) return list[index] = value;
	throw "index out of bounds: the length is " + list.length + " but the index is " + index;
}
function show(plan) {
	let out = "";
	for (const step of plan[0]) {
		const $c = step;
		let $d = null;
		if ($c[0] === 0) {
			const index = $c[1];
			$d = "K" + index + " ";
		} else if ($c[0] === 1) {
			const index2 = $c[1];
			$d = "R" + index2 + " ";
		} else {
			$d = "F ";
		}
		const rendered = $d;
		out = out + rendered;
	}
	out = out + "| removed:";
	for (const index3 of plan[1]) {
		out = out + (" " + index3);
	}
	console.log(out);
}
function $a(old_keys, old_items, items, key_of) {
	let claimed = [  ];
	for (const _ of old_keys) {
		claimed.push(false);
	}
	let steps = [  ];
	for (const item of items) {
		const item_key = key_of(item);
		let step = [ 2 ];
		let index = 0;
		while (index < old_keys.length) {
			if (!(__at(claimed, index)) && __at(old_keys, index) === item_key) {
				__at_put(claimed, index, true);
				let $b = null;
				if (__at(old_items, index) === item) {
					$b = [ 0, index ];
				} else {
					$b = [ 1, index ];
				}
				step = $b;
				break;
			}
			index = index + 1;
		}
		steps.push(step);
	}
	let removed = [  ];
	let index2 = 0;
	while (index2 < old_keys.length) {
		if (!(__at(claimed, index2))) {
			removed.push(index2);
		}
		index2 = index2 + 1;
	}
	return [ steps, removed ];
}
show($a([ 1, 2, 3 ], [ 10, 20, 30 ], [ 30, 10, 20 ], (item) => {
	return Math.trunc(item / 10);
}));
show($a([ 1, 2 ], [ 10, 20 ], [ 10, 21, 35 ], (item) => {
	return Math.trunc(item / 10);
}));
show($a([ 1, 2, 3 ], [ 10, 20, 30 ], [ 30 ], (item) => {
	return Math.trunc(item / 10);
}));
show($a([ 1 ], [ 10 ], [ 10, 10 ], (item) => {
	return Math.trunc(item / 10);
}));
show($a([  ], [  ], [ 10 ], (item) => {
	return Math.trunc(item / 10);
}));
show($a([ 1 ], [ 10 ], [  ], (item) => {
	return Math.trunc(item / 10);
}));
