# date: reference

Calendar dates, times of day and ISO 8601 text. There is no clock and no time zone (`std.time` has the scheduler clock only). Pure Mote; proleptic Gregorian calendar, any `Int` year.

`import { Date, TimeOfDay, DateTime, Weekday, is_leap_year, days_in_month } from date`

| Type | Make | Read / change |
|---|---|---|
| `Date` | `Date.new(y, m, d) -> Date?` (null for a day that does not exist), `Date.from_days(days)` (day 0 is 1970-01-01), `Date.parse("YYYY-MM-DD")` | `.year() .month() .day() .days() .weekday() .add_days(n) .days_since(earlier)`; `to_string` `YYYY-MM-DD` |
| `TimeOfDay` | `TimeOfDay.new(h, m, s) -> TimeOfDay?`, `from_seconds(n)` (wraps at a day), `parse("HH:MM:SS")` | `.hour() .minute() .second() .seconds()`; `HH:MM:SS` |
| `DateTime` | `DateTime.new(date, time)`, `from_seconds(n)` (since 1970-01-01T00:00:00), `parse("YYYY-MM-DDTHH:MM:SS")` | `.date() .time() .seconds() .add_seconds(n)`; `YYYY-MM-DDTHH:MM:SS` |
| `Weekday` | `Monday` ... `Sunday` | `.number()` 1 to 7; prints the name |
| helpers | `is_leap_year(y)`, `days_in_month(y, m)` | |

All are `Eq`, `Ord` and `Display`. `parse` takes only the fixed-width forms and gives an `Err` otherwise. Arithmetic is on days or seconds; there are no months-added or durations types.

```mote
import { Date } from date

fn main() -> Result<Null, Error> {
    let day = Date.parse("2026-10-31")?
    println(day.weekday())
    println(day.add_days(30))
    return Ok(null)
}
```

Tests: `tools/test-core.sh date`.
