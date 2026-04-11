
String expression
	Parse basic
	Parse tags
	Parse escape sequences
	Parse interpolation
	Analyze
	Transform
Number expression
	Parse digits
	Parse format modifiers
	Parse tags
	Analyze
	Transform
Boolean
	Parse
	Analyze
	Transform
Binary expression
	Parse
	Analyze
	Transform
Unary expression
	Parse
	Analyze
	Transform
If/else expression
	Parse
	Analyze
	Transform
Block expression
	Parse
	Analyze
	Transform
Tuple expression
	Parse
	Analyze
	Transform
Match expression
	Parse
	Analyze
	Transform
Variables
	Parse
		Parse basic
		Parse type
	Analyze
		Analyze value type
		Analyze type annotation
		Reconcile value vs annotation type
		Count references
	Transform
		Drop unreferenced variables
Functions
	Declaration
		Name
		Generics
		Parameters
			Name
			Type
		Body
		Return type
	Analyze
	Transform
		Drop unreferenced functions
	Nested functions
		Transform
Function call
	Expression
	Analyze
	Transform
Closures
	Parse
	Analyze
	Transform
Structs
	Declaration
	
	Fields
	Methods (with static-dispatch)
Implementations
Context provider / system
Memory / object disposal
References
Importing
Standard library
	std::io
	std::fs
