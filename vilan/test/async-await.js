import { setTimeout } from "node:timers/promises";
async function labelled(label) {
	await (setTimeout(0));
	return label;
}
(async () => {
	console.log(await (labelled("first")));
	const pending = (async () => {
		return await (labelled("second"));
	})();
	console.log(await (pending));
	const block = (async () => {
		await (setTimeout(0));
		return "third";
	})();
	console.log(await (block));
})();
