function __at(list, index) {
	if (index >= 0 && index < list.length) return list[index];
	throw "index out of bounds: the length is " + list.length + " but the index is " + index;
}
function __at_view(list, index) {
	if (index >= 0 && index < list.length) return [ list, index ];
	throw "index out of bounds: the length is " + list.length + " but the index is " + index;
}
function next_mut(self) {
	let $a = null;
	if (self[1] < self[0].length) {
		const index = self[1];
		self[1] = self[1] + 1;
		$a = [ 0, __at_view(self[0], index) ];
	} else {
		$a = [ 1 ];
	}
	return $a;
}
let counter = [ [ 1, 2, 3 ], 0 ];
const $b = counter;
while (true) {
	const $c = next_mut($b);
	if ($c[0] !== 0) {
		break;
	}
	const element = $c[1];
	element[0][element[1]] = element[0][element[1]] * 10;
}
console.log(__at(counter[0], 0));
console.log(__at(counter[0], 1));
console.log(__at(counter[0], 2));
