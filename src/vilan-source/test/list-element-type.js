function e/*default*/() {

}
function o/*default*/() {
	return 0;
}
function b(c) {
	let d/*total*/ = e/*default*/();
	let f/*seeded*/ = false;
	for (const g/*item*/ of c) {
		if (f/*seeded*/) {
			d/*total*/ = d/*total*/ + g/*item*/;
		} else {
			d/*total*/ = g/*item*/;
			f/*seeded*/ = true;
		}
	}
	return d/*total*/;
}
function h(i) {
	let j/*total*/ = e/*default*/();
	let k/*seeded*/ = false;
	for (const l/*item*/ of i) {
		if (k/*seeded*/) {
			j/*total*/ = j/*total*/ * l/*item*/;
		} else {
			j/*total*/ = l/*item*/;
			k/*seeded*/ = true;
		}
	}
	return j/*total*/;
}
function n(c) {
	let d/*total*/ = o/*default*/();
	let f/*seeded*/ = false;
	for (const g/*item*/ of c) {
		if (f/*seeded*/) {
			d/*total*/ = d/*total*/ + g/*item*/;
		} else {
			d/*total*/ = g/*item*/;
			f/*seeded*/ = true;
		}
	}
	return d/*total*/;
}
let a/*numbers*/ = [  ];
a/*numbers*/.push(2);
a/*numbers*/.push(3);
a/*numbers*/.push(4);
console.log(b(a/*numbers*/));
console.log(h(a/*numbers*/));
const m/*empty*/ = [  ];
console.log(n(m/*empty*/));
for (const p/*n*/ of a/*numbers*/) {
	console.log(p/*n*/);
}
