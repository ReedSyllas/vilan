function $a(self) {
	const $b = self;
	let $c = null;
	if ($b[0] === 0) {
		const x = $b[1][0];
		const y = $b[1][1];
		$c = [ [ 0, x ], [ 0, y ] ];
	} else {
		$c = [ [ 1 ], [ 1 ] ];
	}
	return $c;
}
const pair = [ 0, [ 3, 7 ] ];
console.log($a(pair));
const empty = [ 1 ];
console.log($a(empty));
