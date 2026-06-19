function __args() {
	return process.argv.slice(2);
}
function __env(key) {
	const value = process.env[key];
	return value === undefined ? [ 1 ] : [ 0, value ];
}
function $c(self, fallback) {
	const $d = self;
	let $e = null;
	if ($d[0] === 0) {
		const x = $d[1];
		$e = x;
	} else {
		$e = fallback;
	}
	return $e;
}
const arguments = __args();
console.log(arguments.length);
const $a = __env("VILAN_TEST_VAR");
let $b = null;
if ($a[0] === 0) {
	const value = $a[1];
	$b = console.log(value);
} else {
	$b = console.log("unset");
}
$b;
console.log($c(__env("DEFINITELY_NOT_SET_XYZ"), "unset"));
