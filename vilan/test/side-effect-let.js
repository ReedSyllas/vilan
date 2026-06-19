function c/*bump*/(d) {
	d.push(1);
	return d.length;
}
let a/*xs*/ = [  ];
const b/*a*/ = c/*bump*/(a/*xs*/);
c/*bump*/(a/*xs*/);
console.log(b/*a*/);
console.log(a/*xs*/.length);
