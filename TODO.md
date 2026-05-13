
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
	[x] Parse
		[x] Parse basic
		[x] Parse type
	[ ] Analyze
		[x] Analyze value type
		[x] Analyze type annotation
		[x] Reconcile value vs annotation type
		[x] Count references
	[ ] Transform
		[x] Basic 1:1 transform
		[x] Drop unreferenced variables

Functions
	[ ] Declaration
		[x] Name
		[ ] Generics
		[x] Parameters
			[x] Name
			[x] Type
		[x] Body
		[ ] Return type
	[ ] Analyze
		[x] Simple analyze
	[ ] Transform
		[x] Basic 1:1 transform
		[ ] Drop unreferenced functions
	[ ] Nested functions with enclosed values
		[ ] Transform

Function call
	[x] Expression
	[x] Analyze
	[ ] Transform
		[x] Basic 1:1 transform

Closures
	[ ] Parse
	[ ] Analyze
	[ ] Transform

Structs
	[x] Declaration
	[x] Fields
	[x] Methods (with static-dispatch)

Implementations
	[x] Parse
	[x] Analyze
	[x] Transform

Context provider / system

Memory / object disposal

References

Importing

Standard library
	[ ] std::io
	[ ] std::fs
