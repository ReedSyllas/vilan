import { setTimeout } from "node:timers/promises";
async function delayed(label, ms) {
	await (setTimeout(ms));
	return label;
}
(async () => {
	let promises = [  ];
	promises.push((async () => {
		return await (delayed("a", 20));
	})());
	promises.push((async () => {
		return await (delayed("b", 10));
	})());
	promises.push((async () => {
		return await (delayed("c", 30));
	})());
	const results = await (Promise.all(promises));
	for (const result of results) {
		console.log(result);
	}
})();
