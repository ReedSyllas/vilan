# Spec §2 — Lexical structure

Source text is UTF-8. Lexing converts it to a token stream; **trivia**
(whitespace and comments) separates tokens and is otherwise discarded. A
file consisting only of trivia lexes to an empty token stream (and parses
as an empty module).

## 2.1 Comments

A comment begins with `//` and runs to the end of the line. There are no
block comments.

## 2.2 Identifiers and keywords

```text
IDENT = ascii_letter | "_" , { ascii_letter | digit | "_" } ;
```

Identifiers are ASCII. The following words are **reserved** — they lex as
keyword tokens and are never `IDENT`:

```text
async  await  borrows  const  else    enum   export  external
for    fun    if       impl  import  in     is      jump
let    macro  match    mod   mut     null   own     ret
struct trait  type     use   with    true   false
```

(`true`/`false` lex as boolean literals; `null` as the null literal.)

**Contextual keywords** lex as `IDENT` and take meaning only by position:
`context` (the clause after a closure type, §3.9), `void` (the unit
value/type), `self` and `Self` (receiver and receiver type), `derive`,
`service`, `extern`, `must_use`, `rpc`, `trait_only`, `doc`, `expose`
(attribute names in `[...]` position), and jump targets (`break`,
`continue`) after `jump`. All remain usable as ordinary identifiers
elsewhere.

## 2.3 Literals

### Numbers

```text
NUMBER = decimal | hex ;
decimal = digits , [ "." , digits ] , [ SUFFIX ] ;
hex     = "0x" , hexdigit , { hexdigit } , [ SUFFIX ] ;
SUFFIX  = IDENT   (* immediately adjacent, no space *)
```

The suffix names the literal's type: `i8 i16 i32 u8 u16 u32` (that
two's-complement width), `i53`/`u53` (the wide integers — see below),
`f` (`f64`), `f32`, `f64`, `n` (`BigInt`). An **unknown suffix is a
compile error** (the retired `i64`/`u64` suffixes get a rename hint). An
unsuffixed integer literal is `i32`; an unsuffixed fractional literal is
`f64`. Every integer literal is **range-checked** against its type at
compile time.

`i53` spans the symmetric range ±2^53 and `u53` spans [0, 2^53] — the
window in which every integer is exactly representable in an IEEE-754
double (the backing representation). The names deliberately follow
JavaScript's safe-integer convention (53 bits of integer precision)
rather than the two's-complement `iN` convention; there is no `i64` —
integers beyond the window take `BigInt`.

In a hex literal the digit run is maximal, so a suffix must begin with a
non-hex letter: `0xFFu8` is valid; `0xFFf` is a single hex number `0xFFF`,
not `0xFF` with suffix `f`.

### Strings

```text
STRING           = '"' , { string_char } , '"' ;
string_char      = "\" any_char | any_char_except_quote_backslash ;
MULTILINE_STRING = '"""' , raw_text , '"""' ;
```

In a plain string a backslash escapes the next character; escape sequences
are preserved in the token and interpreted at code generation with
JavaScript string-escape semantics (`\n`, `\"`, `\\`, …). A multiline
string is **raw** (a backslash is a backslash) and runs to the first
`"""`; the whitespace prefix of the line containing the closing delimiter
is stripped from every line of the content.

### Interpolated strings

`i"…"` is an interpolated string: `{expr}` holes embed expressions; `\{`
and `\}` are literal braces. The construct is defined by desugaring — an
interpolated string is exactly equivalent to a parenthesized
concatenation:

```text
i"Hello, {name}!"   ≡   ("" + "Hello, " + (name) + "!")
```

Each hole's contents are lexed as ordinary tokens (except that `{`/`}`
delimit the hole; string literals inside a hole may still contain braces)
and parsed as a single parenthesized expression. The result of the whole
form is `str`; each part must therefore be valid as a `+` operand with
`str` (§5's operator dispatch).

*Implementation note: because a hole is re-lexed as ordinary tokens, a
string literal inside a hole cannot use `\"` escapes — nested quoting
inside holes is currently a parse error. Bind the value to a local first.*

### Other literals

`true`, `false` (type `bool`); `null` (the host-boundary null, §5.2);
`void` (the unit value — a contextual identifier, not a keyword).

## 2.4 Operators and punctuation

Two token classes:

- **Operator tokens** — a maximal run of the characters `- : ! * / + = | &
  ^ ? %`, plus the arrow `=>` (lexed as one token). Maximal munch means
  `==`, `!=`, `+=`, `::`, `?.`'s `?`, `&&`, `||` each lex as single
  operator tokens; conversely `a+-b` lexes as `a`, `+-`, `b` and is a parse
  error.
- **Control tokens** — the single characters `( ) [ ] { } < > ; , .`.

`<` and `>` are control tokens (they delimit generics), not operator
characters. Consequently `<=`/`>=` lex as `<`/`>` followed by `=`, and the
shift operators `<<`/`>>` are two adjacent control tokens; the parser
accepts them as shifts only when **span-adjacent** (no whitespace):
`a << b` is a shift, `a < < b` is a parse error (§3.7).

## 2.5 Trivia and token separation

Whitespace (any Unicode whitespace) and comments may appear between any
two tokens and are required only where two tokens would otherwise lex as
one (`fun main` needs the space; `a + b` does not). Lexing is greedy and
context-free: no token depends on parse state.
