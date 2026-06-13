
String expression
	[x] Parse basic
	[ ] Parse tags
	[ ] Parse escape sequences
	[ ] Parse interpolation
	[x] Analyze
	[x] Transform

Number expression
	[x] Parse digits
	[ ] Parse format modifiers
	[ ] Parse tags
	[x] Analyze
	[x] Transform

Boolean
	[x] Parse
	[x] Analyze
	[x] Transform

Binary expression
	[x] Parse
	[x] Analyze
	[x] Transform

Unary expression
	[ ] Parse
	[ ] Analyze
	[ ] Transform

If/else expression
	[x] Parse
	[x] Analyze
	[x] Transform

Block expression
	[x] Parse
	[ ] Analyze
	[ ] Transform

Tuple expression
	[x] Parse
	[x] Analyze
	[x] Transform

Match expression
	[ ] Parse
	[ ] Analyze
	[ ] Transform

Variables
	[x] Parse basic
	[x] Parse type
	[x] Analyze value type
	[x] Analyze type annotation
	[x] Reconcile value vs annotation type
	[x] Count references
	[x] Basic 1:1 transform
	[x] Drop unreferenced variables

Functions
	[ ] Declaration
		[x] Name
		[ ] Generics
		[x] Parameter name
		[x] Parameter type
		[x] Body
		[x] Return type
	[x] Simple analyze
	[x] Basic 1:1 transform
	[x] Drop unreferenced functions
	[ ] Nested functions with enclosed values

Function call
	[x] Expression
	[x] Analyze
	[x] Parameter count checking
	[x] Parameter type checking
	[x] Basic 1:1 transform

Closures
	[x] Parse
	[x] Analyze
	[x] Transform

Structs
	[x] Declaration
	[x] Fields
	[x] Methods (with static-dispatch)

Implementations
	[x] Parse
	[x] Analyze
	[x] Transform

Context provider / system (based on NodeJS `AsyncLocalContext` API)

Memory / object disposal

References

Importing
	[x] Parse
	[x] Analyze
	[x] Transform

Standard library
	[-] std::io
	[ ] std::fs
