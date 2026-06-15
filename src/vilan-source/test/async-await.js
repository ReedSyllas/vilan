import { setTimeout } from "node:timers/promises";
async function a/*labelled*/(b) {
	await (setTimeout(0));
	return b;
}
(async () => {
	console.log(await (a/*labelled*/("first")));
	const c/*pending*/ = (async () => {
	return await (a/*labelled*/("second"));
})();
	console.log(await (c/*pending*/));
	const d/*block*/ = (async () => {
	await (setTimeout(0));
	return "third";
})();
	console.log(await (d/*block*/));
})();
