function describe(order) {
	const $a = order;
	let $b = null;
	if ($a === -1) {
		$b = "less";
	} else if ($a === 0) {
		$b = "equal";
	} else {
		$b = "greater";
	}
	return $b;
}
console.log(-1);
console.log(1);
console.log(-1 < 0);
console.log(1 < 0);
console.log(0 === 0);
console.log(describe(1));
console.log(describe(-1));
