function new2(n) {
	return [ n ];
}
function default2() {
	return new2(0);
}
function $a() {
	return default2();
}
const some_id = $a();
console.log(some_id);
