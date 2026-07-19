function drop(self) {
	console.log(self[0]);
}
function $b($c) {
	drop($c);
}
let $a = undefined;
const guard = [ "teardown" ];
try {
	console.log("body");
	$a = 7;
} finally {
	$b(guard);
}
process.exit($a);
