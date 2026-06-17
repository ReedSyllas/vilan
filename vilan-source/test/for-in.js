let a/*names*/ = [  ];
a/*names*/.push("Anna");
a/*names*/.push("James");
a/*names*/.push("Roger");
for (const b/*name*/ of a/*names*/) {
	console.log(b/*name*/);
}
let c/*numbers*/ = [  ];
c/*numbers*/.push(1);
c/*numbers*/.push(2);
c/*numbers*/.push(3);
c/*numbers*/.push(4);
for (const d/*number*/ of c/*numbers*/) {
	if (d/*number*/ === 3) {
		continue;
	}
	if (d/*number*/ === 4) {
		break;
	}
	console.log(d/*number*/);
}
let e/*count*/ = 0;
for (const _ of a/*names*/) {
	e/*count*/ = e/*count*/ + 1;
}
console.log(e/*count*/);
