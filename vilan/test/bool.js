function describe(flag2) {
	const $a = flag2;
	let $b = null;
	if ($a === true) {
		$b = "yes";
	} else {
		$b = "no";
	}
	return $b;
}
const flag = true;
const computed = 3 < 5;
console.log(flag);
console.log(flag && computed);
console.log(!(flag));
console.log(!(9 > 5));
console.log(describe(flag));
console.log(describe(9 > 5));
const $c = flag;
console.log($c === true);
