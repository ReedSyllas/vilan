function m/*default*/() {

}
function c/*next*/(d) {
	a/*produced*/ = a/*produced*/ + 1;
	let e = null;
	if (a/*produced*/ <= d[0]) {
		e = [ 0, a/*produced*/ ];
	} else {
		e = [ 1 ];
	}
	return e;
}
function j(k) {
	let l/*total*/ = m/*default*/();
	let n/*seeded*/ = false;
	for (const o/*item*/ of k) {
		if (n/*seeded*/) {
			l/*total*/ = l/*total*/ + o/*item*/;
		} else {
			l/*total*/ = o/*item*/;
			n/*seeded*/ = true;
		}
	}
	return l/*total*/;
}
function p(q) {
	let r/*total*/ = m/*default*/();
	let s/*seeded*/ = false;
	for (const t/*item*/ of q) {
		if (s/*seeded*/) {
			r/*total*/ = r/*total*/ * t/*item*/;
		} else {
			r/*total*/ = t/*item*/;
			s/*seeded*/ = true;
		}
	}
	return r/*total*/;
}
let a/*produced*/ = 0;
const b/*naturals*/ = [ 3 ];
const f = b/*naturals*/;
while (true) {
	const g = c/*next*/(f);
	if (g[0] !== 0) {
		break;
	}
	const h/*n*/ = g[1];
	console.log(h/*n*/);
}
let i/*numbers*/ = [  ];
i/*numbers*/.push(2);
i/*numbers*/.push(3);
i/*numbers*/.push(4);
console.log(j(i/*numbers*/));
console.log(p(i/*numbers*/));
