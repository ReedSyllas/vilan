# Compiler Bindings

An api for hooking with the compiler in user projects to extend the core functionality and interoperability of the compiler.

## Compiler bindings API

`compiler/lib.vl`:
```vilan
struct string;

struct Struct {
	name: string,
	fields: List<StructField>,
}

struct StructField {
	name: string,
	type: Type,
}

enum Type {
	// ...
}
```

## User code usage

```vilan
macro {
	// Macro blocks share the same syntax as other Vilan code.
	// However, they share a different scope and context.
	// Outside code cannot be accessed from inside of here. Only the compiler bindings api is available.
	
	for name in [ "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f32", "f64" ] {
		// `macro` is a handle for the current macro block. Here is it used to insert items in place of where the `macro` block is.
		macro::insert(
			Struct {
				name,
				fields = [],
			}
		);
	}
}
```

The above is equivalent to:
```
struct u8;
struct u16;
struct u32;
struct u64;
struct i8;
struct i16;
struct i32;
struct i64;
struct f32;
struct f64;
```

Other uses include building custom attributes (attributes are not implemented yet).

```vilan
// Syntax not finalized.
attribute constructor for struct {
	// This has the same rules as the `macro` block.
	// No access to outside resources. Only the bindings api is available.
	
	// `struct` is a handle for the struct that this attribute is attached to.
	struct::insert_after(
		Impl {
			name = struct::name,
			body = [
				Method {
					name = "new",
					parameters = struct::fields.map(|field| Parameter { name = field.name, type = field.type }),
					body = ([], StructInitializer {
						name = struct::name,
						fields = struct::fields.map(|field| StructInitializerField { name = field.name, value = field.name }),
					}),
				}
			],
		}
	);
}

[constructor]
struct Point {
	x: f64,
	y: f64,
}

fun main() {
	let point = Point::new(7, 5);
	// ...
}
```
