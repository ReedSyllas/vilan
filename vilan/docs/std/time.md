# std::time — reference

Instants, durations, and timers. Both types are Wire — they ride rpc
payloads (`created_at: Instant` in a mirrored record is the standard
timestamp shape).

```vilan,fragment
import std::time::{ now, Instant, Duration, sleep, sleep_for };
```

## Instant

A moment in time — epoch milliseconds in an `i53` under the hood.

```vilan,fragment
fun now(): Instant                       // the current wall-clock moment

impl Instant {
	fun since(self, earlier: Instant): Duration   // self - earlier
	fun to_iso(self): str                         // ISO-8601, via the host clock
}
impl Instant with Add<Duration> { … }    // instant + duration → Instant
impl Instant with Sub<Duration> { … }    // instant - duration → Instant
impl Instant with PartialOrd { … }       // <, >, == between instants
```

## Duration

```vilan,fragment
impl Duration {
	// constructors
	fun millis(count: i53): Duration
	fun seconds(count: i53): Duration
	fun minutes(count: i53): Duration
	fun hours(count: i53): Duration
	fun days(count: i53): Duration
	// truncating accessors
	fun as_seconds(self): i53
	fun as_minutes(self): i53
	fun as_hours(self): i53
	fun as_days(self): i53
	// human text: "42 seconds", "3 hours" …
	fun describe(self): str
}
impl Duration with Add { … }             // duration + duration
impl Duration with Sub { … }
impl Duration with PartialOrd { … }
```

The `"{age} ago"` UI idiom:

```vilan
import std::print;
import std::time::{ now, Instant, Duration };

fun main() {
	let started = now();
	let deadline = started + Duration::hours(2i53);
	print(deadline.since(started).describe());
	print(started < deadline);
}
```

## Timers

```vilan,fragment
fun sleep(ms: i32)                  // suspend (async; callers implicitly await)
fun sleep_for(duration: Duration)
fun set_timeout(callback: || void, ms: i32)   // host setTimeout, fire-and-forget
```

## Notes

- Duration constructors take `i53` — remember the suffix on computed
  literals (`Duration::millis(500i53 * factor)`); see
  [gotchas](../appendix/gotchas.md).
- `now()` is a host call, so it can't be `const`-folded, and programs using
  it aren't output-deterministic — keep it out of golden-file tests.
- Wire format: an `Instant` serializes as its `i53` millis — exact for any
  realistic date (i53 rides the wire as a float's 53 bits, safe past year
  200,000).
