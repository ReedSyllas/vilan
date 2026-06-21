import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
async function read_file_to_str(path2) {
	return await (readFile(path2, "utf8"));
}
function path(self) {
	return self[0].url;
}
function builder() {
	return [ 200, "Content-Type", "text/plain", "" ];
}
function code(self, code2) {
	self[0] = code2;
	return self;
}
function set_header(self, name, value) {
	self[1] = name;
	self[2] = value;
	return self;
}
function body(self, body2) {
	self[3] = body2;
	return self;
}
function build(self) {
	return [ self[0], self[1], self[2], self[3] ];
}
function builder2() {
	return [ 3000, (request) => {
		return build(body(code(builder(), 404), "Not Found"));
	}, (server) => {
		return;
	}, (server) => {
		return;
	} ];
}
function url(self) {
	return "http://localhost:" + self[0] + "/";
}
function start(self) {
	const request_handler = self[1];
	const on_start2 = self[2];
	const server = __clone(self);
	const port2 = self[0];
	const node_server = createServer((node_request, node_response) => {
		const response = request_handler([ node_request ]);
		node_response.statusCode = response[0];
		node_response.setHeader(response[1], response[2]);
		node_response.end(response[3]);
		return;
	});
	node_server.listen(port2, () => {
		on_start2(server);
		return;
	});
}
function port(self, port2) {
	self[0] = port2;
	return self;
}
function on_request(self, handler) {
	self[1] = handler;
	return self;
}
function on_start(self, callback) {
	self[2] = callback;
	return self;
}
function build2(self) {
	return [ self[0], self[1], self[2] ];
}
function page() {
	const style = "<style>body { font: 16px/1.5 system-ui, sans-serif; max-width: 32rem; margin: 2rem auto; } section { margin-block: 2rem; } button { margin-right: 0.25rem; } .todos .row { display: flex; gap: 0.5rem; } .todos ul { list-style: none; padding: 0; } .todos li { display: flex; gap: 0.5rem; align-items: center; } .todos li.done span { text-decoration: line-through; opacity: 0.5; } .todos .remove { margin-left: auto; } .todos .filters .active { font-weight: bold; } .todos .empty { opacity: 0.6; } </style>";
	return "<!doctype html><html><head><meta charset=\"utf-8\"><title>Vilan full-stack</title>" + style + "</head><body><div id=" + "\"" + "app" + "\"" + "></div><script type=" + "\"" + "module" + "\"" + " src=" + "\"" + "/client.js" + "\"" + "></script></body></html>";
}
(async () => {
	const client_js = await (read_file_to_str("dist/client.js"));
	const server = build2(on_start(on_request(port(builder2(), 59386), (request) => {
		const $a = path(request);
		let $b = null;
		if ($a === "/client.js") {
			$b = build(body(set_header(builder(), "Content-Type", "text/javascript"), client_js));
		} else if ($a === "/api/hello") {
			$b = build(body(set_header(builder(), "Content-Type", "text/plain"), "some api message"));
		} else {
			$b = build(body(set_header(builder(), "Content-Type", "text/html"), page()));
		}
		return $b;
	}), (server2) => {
		console.log("server started");
		console.log(url(server2));
		return;
	}));
	start(server);
})();
