import { createServer } from "node:http";

const PORT = 3000;

createServer((req, res) => {
	res.statusCode = 200;
	res.setHeader("Content-Type", "text/plain");
	res.end("Hello, World!\n");
}).listen(PORT, () => {
	console.log(`Server running at http://localhost:${PORT}/`);
});
