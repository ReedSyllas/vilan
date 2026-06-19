import { setTimeout } from "node:timers/promises";
async function b/*delayed*/(c, d) {
	await (setTimeout(d));
	return c;
}
(async () => {
	let a/*promises*/ = [  ];
	a/*promises*/.push((async () => {
		return await (b/*delayed*/("a", 20));
	})());
	a/*promises*/.push((async () => {
		return await (b/*delayed*/("b", 10));
	})());
	a/*promises*/.push((async () => {
		return await (b/*delayed*/("c", 30));
	})());
	const e/*results*/ = await (Promise.all(a/*promises*/));
	for (const f/*result*/ of e/*results*/) {
		console.log(f/*result*/);
	}
})();
