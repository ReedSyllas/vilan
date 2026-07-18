import { setTimeout } from "node:timers/promises";
class __Task {
	constructor(run, origin, nursery) {
		this.origin = origin;
		this.observed = false;
		this.nursery = nursery;
		this.owned = !!nursery;
		this.rejected = false;
		this.error = undefined;
		this.promise = run();
		this.promise.then(null, (error) => {
			this.rejected = true;
			this.error = error;
			if (this.owned && !__nursery_is_cancel(error)) this.nursery.__fail(this);
			if (!this.observed && !this.owned) {
				globalThis.setTimeout(() => {
					if (!this.observed) console.error("unhandled task error (spawned in " + this.origin + "): " + String(error));
				}, 0);
			}
		});
		if (nursery) nursery.children.push(this);
	}
	then(onFulfilled, onRejected) {
		this.observed = true;
		return this.promise.then(onFulfilled, onRejected);
	}
}
function __task(run, origin, nursery) {
	return new __Task(run, origin, nursery);
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
