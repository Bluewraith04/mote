# process: reference

Run programs and talk to them. Native: Zig (`fork`/`exec`-style spawn, pipes, `waitpid`).

`import { command, Command, Child, Stdio, Output, Status } from process`

## Command

`command(program)` makes a `Command` with no arguments whose streams are this program's. `Command` is a class; every method gives a changed copy, so chains are safe. The program is found on `PATH` when its name has no `/`.

| Method | Notes |
|---|---|
| `.arg(text)` / `.with_args(list)` | add arguments |
| `.set_env(name, value)` | a variable for the child, replacing one of that name |
| `.without_inherited_env()` | the child gets only what `set_env` set |
| `.in_dir(path)` | working directory of the child |
| `.with_stdin(s)` `.with_stdout(s)` `.with_stderr(s)` | `Stdio.Inherit` (this program's), `Piped` (a pipe you use), `Null` (the null device), `Merged` (stderr only: into stdout) |
| `.status() -> Result<Status, Error>` | run to the end with the streams as they are |
| `.output() -> Result<Output, Error>` | no input; collects stdout and stderr, waits |
| `.output_with(input: [UInt8])` | the same with `input` as standard input |
| `.start() -> Result<Child, Error>` | start and keep the pipes you asked for |

`Output { status, stdout: [UInt8], stderr: [UInt8] }` with `.stdout_string()` and `.stderr_string()` (`String?`, null if not UTF-8). `Status { raw }`: `.code() -> Int?` (null if a signal ended it), `.signal() -> Int?`, `.success()`, prints as `exit code 0` or `signal 9`.

## Child

`Child` is `Closeable`: closing closes the pipes, it does not wait.

| Method | Notes |
|---|---|
| `.id()` | the process id |
| `.write(bytes)`, `.write_string(text)`, `.close_stdin()` | needs `with_stdin(Stdio.Piped)`; close it so the child sees the end of input |
| `.read(count)`, `.read_all()` | standard output (needs `Piped`) |
| `.read_stderr(count)`, `.read_all_stderr()` | standard error (needs `Piped`) |
| `.wait() -> Result<Status, Error>` | waiting twice gives the same status |
| `.try_wait() -> Result<Status?, Error>` | null while running |
| `.signal(number)`, `.kill()` | 15 asks it to end, 9 (`kill`) ends it |
| `.close()` | |

A pipe not asked for is an `Err` when used. Reading both pipes of a chatty child from one task can deadlock on a full pipe; read the second from another task or use `output()`.

## Examples

```mote
import { command, Stdio } from process

fn main() -> Result<Null, Error> {
    let out = command("echo").arg("hello").output()?
    println(out.stdout_string()!)
    println(out.status.success())

    let child = command("cat").with_stdin(Stdio.Piped).with_stdout(Stdio.Piped).start()?
    child.write_string("through a pipe\n")?
    child.close_stdin()?
    println(String.from_bytes(child.read_all()?)!)
    let status = child.wait()?
    println(status.success())
    return Ok(null)
}
```

Tests: `tools/test-core.sh process`.
