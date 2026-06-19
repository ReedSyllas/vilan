function is_empty(self) {
	return self.length === 0;
}
const s = "Hello, World";
console.log(s.length);
console.log(s.includes("World"));
console.log(s.startsWith("Hello"));
console.log(s.endsWith("!"));
console.log(s.toUpperCase());
console.log(s.replaceAll("o", "0"));
console.log(s.substring(0, 5));
console.log("ab".repeat(3));
console.log(is_empty("  hi  ".trim()));
console.log(is_empty(""));
for (const part of "a,b,c".split(",")) {
	console.log(part);
}
