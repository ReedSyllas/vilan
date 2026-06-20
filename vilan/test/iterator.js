function $a(fn) {
	return [ fn ];
}
function $b(self) {
	return self[0]();
}
let i = 0;
const naturals = $a(() => {
	i = i + 1;
	return i;
});
console.log($b(naturals));
console.log($b(naturals));
console.log($b(naturals));
