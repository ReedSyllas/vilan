import { setTimeout } from "node:timers/promises";
async function b/*delayed*/(c, d) {
	await (setTimeout(d));
	return c;
}
(async () => {
	const a/*promises*/ = [  ];
	a/*promises*/.push((async () => {
	return await (b/*delayed*/("a", 20));
})());
	a/*promises*/.push((async () => {
	return await (b/*delayed*/("b", 10));
})());
	a/*promises*/.push((async () => {
	return await (b/*delayed*/("c", 30));
})());
	for (const e/*result*/ of await (Promise.all(a/*promises*/))) {
		console.log(e/*result*/);
	}
})();
