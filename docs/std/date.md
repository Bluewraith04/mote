# std.date

Calendar dates and times with fixed UTC offsets and the machine's offset. Named time zones are not supported. `import std.date as date`.

## `Date`

`Date.new(year, month, day)` answers a `Result`; `Date.from_days(n)` is `n` days after 1970-01-01.

| Method | Answers |
|---|---|
| `days_since_epoch()`, `day_of_year()` | as named |
| `weekday()` | 0 for Monday to 6 for Sunday |
| `add_days(n)`, `add_months(n)`, `add_years(n)` | a new date; the day clamps to a shorter month |
| `days_until(other)`, `compare(other)` | difference; `-1`, `0` or `1` |
| `to_iso()` | `2024-03-09` |

```mote
import { Date } from std.date

let d = Date.new(2024, 3, 9)!
println(d.add_months(1).to_iso())
println(d.weekday())
```

```output
2024-04-09
5
```

## `DateTime`

A civil date and time with `offset`, seconds east of UTC.

| Item | Answers |
|---|---|
| `DateTime.new(year, month, day, hour, minute, second, nano, offset)` | a checked `Result` |
| `DateTime.from_unix(secs, nano, offset)`, `from_unix_millis(ms, offset)` | the moment shown at `offset` |
| `date()`, `unix_secs()`, `unix_millis()` | parts and epoch values |
| `with_offset(offset)`, `to_local()` | the same moment at another offset; `to_local` is a `Result` |
| `plus(d)`, `since(earlier)`, `compare(other)` | shift and measure with a `Duration`; compare moments |
| `to_iso()` | `2024-03-09T12:30:00Z` |
| `format(pattern)` | fills `%Y %m %d %H %M %S %f %j %a %A %b %B %e %z %%` |

## Functions

| Function | Answers |
|---|---|
| `parse_date(text)` | `Result` of a `Date` from `YYYY-MM-DD` |
| `parse_iso(text)` | `Result` of a `DateTime`; a missing offset means UTC |
| `now_utc()`, `now_local()` | the current moment; `now_local` is a `Result` |
| `local_offset(unix_secs)` | the machine's offset at that moment, as a `Result` |
| `is_leap_year(y)`, `days_in_month(y, m)`, `weekday_name(n)`, `month_name(m)` | as named |
