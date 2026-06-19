function from_fn(fn) {
	return [ fn ];
}
function $a(self) {
	return self[0]();
}
let i = 0;
const naturals = from_fn(() => {
	i = i + 1;
	return i;
});
console.log($a(naturals));
console.log($a(naturals));
console.log($a(naturals));
