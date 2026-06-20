function __shared_new(value) {
	return { v: value };
}
function new2(value) {
	return [ __shared_new(value) ];
}
function $b(self) {
	return self[0].v;
}
function $c(self, value) {
	self[0].v = value;
}
function $a(self, transform) {
	$c(self, transform($b(self)));
}
const counter = new2(0);
$a(counter, (n) => {
	return n + 1;
});
$a(counter, (n) => {
	return n * 10;
});
console.log($b(counter));
const label = new2("a");
$c(label, "hello");
console.log($b(label));
