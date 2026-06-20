function __shared_new(value) {
	return { v: value };
}
function $a(value) {
	return [ __shared_new(value) ];
}
function $c(self) {
	return self[0].v;
}
function $d(self, value) {
	self[0].v = value;
}
function $b(self, transform) {
	$d(self, transform($c(self)));
}
function $e(value) {
	return [ __shared_new(value) ];
}
function $f(self, value) {
	self[0].v = value;
}
function $g(self) {
	return self[0].v;
}
const counter = $a(0);
$b(counter, (n) => {
	return n + 1;
});
$b(counter, (n) => {
	return n * 10;
});
console.log($c(counter));
const label = $e("a");
$f(label, "hello");
console.log($g(label));
