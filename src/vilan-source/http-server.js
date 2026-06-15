import { createServer } from "node:http";
function e/*builder*/() {
	return [ 200, "Content-Type", "text/plain", "" ];
}
function f/*code*/(g, h) {
	return [ h, g[1], g[2], g[3] ];
}
function i/*set_header*/(j, k, l) {
	return [ j[0], k, l, j[3] ];
}
function m/*body*/(n, o) {
	return [ n[0], n[1], n[2], o ];
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
	const W/*server*/ = T;
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
	return [ P, O[1], O[2], O[3] ];
}
function K/*on_request*/(L, M) {
	return [ L[0], M, L[2], L[3] ];
}
function C/*on_start*/(D, E) {
	return [ D[0], D[1], E, D[3] ];
}
function G/*on_stop*/(H, I) {
	return [ H[0], H[1], H[2], I ];
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
		u/*builder*/ = K/*on_request*/(G/*on_stop*/(C/*on_start*/(u/*builder*/, (z) => {
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
		u/*builder*/ = K/*on_request*/(u/*builder*/, s);
	}
	return Q/*build*/(N/*port*/(u/*builder*/, 3000));
}
const a/*server*/ = r/*create*/((b) => {
	return c/*text*/("Hello, World!");
}, true);
S/*start*/(a/*server*/);
