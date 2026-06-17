function __parse_i32(text) {
	const value = Number.parseInt(text, 10);
	return Number.isNaN(value) ? [ 1 ] : [ 0, value ];
}
function __random_i32(low, high) {
	return Math.floor(Math.random() * (high - low + 1)) + low;
}
let __vilan_stdin = null, __vilan_stdin_index = 0;
function __scan() {
	if (__vilan_stdin === null) {
		try {
			__vilan_stdin = require("fs").readFileSync(0, "utf-8").split("\n");
		} catch (error) {
			__vilan_stdin = [];
		}
	}
	return __vilan_stdin_index < __vilan_stdin.length ? __vilan_stdin[__vilan_stdin_index++] : "";
}
function l/*diff*/(m, n) {
	let o = null;
	if (m > n) {
		o = m - n;
	} else {
		o = n - m;
	}
	return o;
}
function c/*play*/() {
	const d/*secret*/ = __random_i32(1, 100);
	console.log("I have chosen a secret number from 1 to 100.");
	console.log("Take a guess!");
	while (true) {
		const e = __scan().trim().toLowerCase();
		let f = null;
		if (e === "quit") {
			return [ 0 ];
			f = undefined;
		} else {
			const g/*n*/ = e;
			const h = __parse_i32(g/*n*/);
			let i = null;
			if (h[0] === 0) {
				const j/*n*/ = h[1];
				if (j/*n*/ !== d/*secret*/) {
					const p = l/*diff*/(d/*secret*/, j/*n*/);
					let q = null;
					if (p <= 2) {
						q = "Almost there!";
					} else if (p <= 5) {
						q = "You\'re very close!";
					} else if (p <= 10) {
						q = "You\'re fairly close.";
					} else if (p <= 25) {
						q = "Not bad.";
					} else {
						q = "Not quite.";
					}
					const k/*temperature_hint*/ = q;
					let s = null;
					if (j/*n*/ < d/*secret*/) {
						s = "Try higher.";
					} else {
						s = "Try lower.";
					}
					const r/*direction_hint*/ = s;
					console.log("" + k/*temperature_hint*/ + " " + r/*direction_hint*/);
					continue;
				}
				console.log("That\'s correct!");
				i = undefined;
			} else {
				console.log("I don\'t understand that. Please type a number (like \'45\').");
				continue;
				i = undefined;
			}
			f = i;
		}
		f;
		break;
	}
	return [ 1 ];
}
console.log("Welcome to the number guessing game!");
console.log("Type \'quit\' at any time to quit.");
console.log("Are you ready to play? Y/n");
while (true) {
	const a = __scan().trim().toLowerCase();
	let b = null;
	if (a === "quit") {
		b = undefined;
	} else if (a === "y") {
		const t = c/*play*/();
		let u = null;
		if (t[0] === 0) {
			u = undefined;
		} else {
			console.log("Play again? Y/n");
			continue;
			u = undefined;
		}
		b = u;
	} else if (a === "") {
		const v = c/*play*/();
		let w = null;
		if (v[0] === 0) {
			w = undefined;
		} else {
			console.log("Play again? Y/n");
			continue;
			w = undefined;
		}
		b = w;
	} else {
		console.log("Another time then. Goodbye!");
		b = undefined;
	}
	b;
	break;
}
