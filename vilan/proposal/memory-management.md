# Memory Management

> **Status: superseded** by [`memory-management-rev-1.md`](memory-management-rev-1.md)
> (the four-rules design that shipped). Kept as the original exploration.

```vilan
mut a = 10;
a = 20;
print(i"{a}"); // 20

mut b = a; // copy
b = 30;
print(i"{a}, {b}"); // 20, 30

let c = &a; // create view of `mut a`
*c = 40;    // write back to view target
print(i"{a}, {b}, {c}"); // 40, 30, 40

let d = c; // copy view of `mut a`.
*d = 50;
print(i"{a}, {b}, {c}, {d}"); // 50, 30, 50, 50
```

Structs are treated the same as primitives.

```
mut a = Point::new(7, 5);
mut b = a; // copy

a.x = 10;
b.x = 20;

print(i"{a}, {b}"); // Point { x = 10, y = 5 }, Point { x = 20, y = 5 }
```

Views don't have ownership of their target.

```vilan
fun f1(): i32 {
	let a = 0;
	a // ok
}

fun f2(): &i32 {
	let a = 0;
	&a // `&a` doesn't have ownership of `a` and thus cannot move it out of this scope
}
```

Immutables are optimized when appropriate.
```vilan
mut a = Point::new(4, 6);
mut b = a; // Developer sees: copy. Compiler optimizes to reuse since `a` is used for the last time here (consumed)

b.x = 10;

print(b); // Point { x = 10, y = 6 }
```

The above would compile into something like:
```js
const a = [4, 6];
const b = a; // no copy

b[0] = 10;

console.log(b);
```

While this
```vilan
mut a = Point::new(4, 6);
mut b = a;

b.x = 10;

print(i"{a}, {b}"); // Point { x = 4, y = 6 }, Point { x = 10, y = 6 }
```
would become
```js
const a = [4, 6];
const b = structuredClone(a);

b[0] = 10;

console.log(`${a}, ${b}`);
```

# More examples

```vilan
mut list = [ 1, 2, 3 ];

for &item in list {
    *item *= 10;
}

print(list); // [ 10, 20, 30 ]
```

```vilan
mut val = Some(10);

match val {
	Some(&x) => {
		*x = 20;
	}
	None => {}
}

print(val); // Some(20)
```

```vilan
impl &(type T) {
	fun insert(self, value: T) {
		*self = value;
	}
}

mut a = None;

let b = &a;
b.insert(Some(5));

print(a); // Some(5)
```
