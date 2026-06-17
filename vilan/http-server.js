import { createServer } from "node:http";
function __clone(value) {
	return Array.isArray(value) ? value.map(__clone) : value;
}
function e/*builder*/() {
	return [ 200, "Content-Type", "text/plain", "" ];
}
function f/*code*/(g, h) {
	g[0] = h;
	return g;
}
function i/*set_header*/(j, k, l) {
	j[1] = k;
	j[2] = l;
	return j;
}
function m/*body*/(n, o) {
	n[3] = o;
	return n;
}
function p/*build*/(q) {
	return [ q[0], q[1], q[2], q[3] ];
}
function v/*builder*/() {
	return [ 3000, (w) => {
	return p/*build*/(m/*body*/(f/*code*/(e/*builder*/(), 404), "Not Found"));
}, (x) => {
	return;
}, (y) => {
	return;
} ];
}
function A/*url*/(B) {
	return "http://localhost:" + B[0] + "/";
}
function S/*start*/(T) {
	const U/*request_handler*/ = T[1];
	const V/*on_start*/ = T[2];
	const W/*server*/ = __clone(T);
	const X/*port*/ = T[0];
	const Y/*node_server*/ = createServer((Z, aa) => {
	const ab/*response*/ = U/*request_handler*/([ Z ]);
	aa.statusCode = ab/*response*/[0];
	aa.setHeader(ab/*response*/[1], ab/*response*/[2]);
	aa.end(ab/*response*/[3]);
	return;
});
	Y/*node_server*/.listen(X/*port*/, () => {
	V/*on_start*/(W/*server*/);
	return;
});
}
function N/*port*/(O, P) {
	O[0] = P;
	return O;
}
function K/*on_request*/(L, M) {
	L[1] = M;
	return L;
}
function C/*on_start*/(D, E) {
	D[2] = E;
	return D;
}
function G/*on_stop*/(H, I) {
	H[3] = I;
	return H;
}
function Q/*build*/(R) {
	return [ R[0], R[1], R[2] ];
}
function c/*text*/(d) {
	return p/*build*/(m/*body*/(i/*set_header*/(f/*code*/(e/*builder*/(), 200), "Content-Type", "text/plain"), "" + d + "\n"));
}
function r/*create*/(s, t) {
	let u/*builder*/ = v/*builder*/();
	if (t) {
		u/*builder*/ = K/*on_request*/(G/*on_stop*/(C/*on_start*/(__clone(u/*builder*/), (z) => {
	console.log("server: Server running at " + A/*url*/(z));
	return;
}), (F) => {
	console.log("server: Server stopped");
	return;
}), (J) => {
	console.log("server: Request received");
	return s(J);
});
	} else {
		u/*builder*/ = K/*on_request*/(__clone(u/*builder*/), s);
	}
	return Q/*build*/(N/*port*/(__clone(u/*builder*/), 3000));
}
const a/*server*/ = r/*create*/((b) => {
	return c/*text*/("Hello, World!");
}, true);
S/*start*/(a/*server*/);
