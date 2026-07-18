import { setTimeout } from "node:timers/promises";
class __Task {
	constructor(run, origin) {
		this.origin = origin;
		this.observed = false;
		this.promise = run();
		this.promise.then(null, (error) => {
			if (!this.observed) {
				globalThis.setTimeout(() => {
					if (!this.observed) console.error("unhandled task error (spawned in " + this.origin + "): " + String(error));
				}, 0);
			}
		});
	}
	then(onFulfilled, onRejected) {
		this.observed = true;
		return this.promise.then(onFulfilled, onRejected);
	}
}
function __task(run, origin) {
	return new __Task(run, origin);
}
async function labelled(label) {
	await (setTimeout(0));
	return label;
}
(async () => {
	console.log(await (labelled("first")));
	const pending = __task(async () => {
		return await (labelled("second"));
	}, "main");
	console.log(await (pending));
	const block = __task(async () => {
		await (setTimeout(0));
		return "third";
	}, "main");
	console.log(await (block));
})();
