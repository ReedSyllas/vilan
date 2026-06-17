function f/*sum*/(g) {
	let h/*total*/ = i/*default*/();
	let j/*seeded*/ = false;
	for (const k/*item*/ of g) {
		if (j/*seeded*/) {
			h/*total*/ = h/*total*/ + k/*item*/;
		} else {
			h/*total*/ = k/*item*/;
			j/*seeded*/ = true;
		}
	}
	return h/*total*/;
}
function i/*default*/() {

}
function c/*sum*/(d) {
	return d[0] + d[1];
}
let a/*points*/ = [  ];
a/*points*/.push([ 1, 2 ]);
a/*points*/.push([ 3, 4 ]);
for (const b/*point*/ of a/*points*/) {
	console.log(c/*sum*/(b/*point*/));
}
let e/*numbers*/ = [  ];
e/*numbers*/.push(10);
e/*numbers*/.push(20);
e/*numbers*/.push(30);
console.log(f/*sum*/(e/*numbers*/));
