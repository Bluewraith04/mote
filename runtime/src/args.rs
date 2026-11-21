//! The command line of an executable.

use crate::exec::Feed;

/// What the command line asks for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// The seed of the scheduler.
    pub seed: Option<u64>,
    /// The pushes to the ports the program opens, from `--feed` and `--close`.
    pub feeds: Vec<Feed>,
    /// The arguments after `--`, which the program reads.
    pub program: Vec<String>,
}

/// The number the next argument is, or the text of the problem.
fn number(name: &str, value: Option<&String>) -> Result<u64, String> {
    let text = value.ok_or_else(|| format!("`{name}` needs a value"))?;
    text.parse::<u64>().map_err(|_| format!("`{name}` is a number, not {text}"))
}

/// The bytes of a `--feed`, as decimal numbers split by commas.
fn bytes(text: &str) -> Result<Vec<u8>, String> {
    text.split(',').map(|byte| byte.parse::<u8>().map_err(|_| format!("`--feed` bytes are numbers up to 255, not {byte}"))).collect()
}

/// Reads the arguments after the program's name.
pub fn parse(arguments: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut at = 0;
    while at < arguments.len() {
        let argument = &arguments[at];
        at += 1;
        let (name, inline) = match argument.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => (argument.as_str(), None),
        };
        match (name, inline) {
            ("--", None) => {
                args.program = arguments[at..].to_vec();
                return Ok(args);
            }
            ("--seed", inline) => {
                let value = match inline {
                    Some(value) => Some(value),
                    None => {
                        at += 1;
                        arguments.get(at - 1).cloned()
                    }
                };
                if args.seed.replace(number("--seed", value.as_ref())?).is_some() {
                    return Err("`--seed` is given twice".to_string());
                }
            }
            ("--feed", None) => {
                let given = arguments.get(at..at + 3).ok_or("`--feed` needs a port, a time and bytes")?;
                at += 3;
                args.feeds.push(Feed { port: number("--feed", given.first())? as usize, at_ms: number("--feed", given.get(1))?, bytes: Some(bytes(&given[2])?) });
            }
            ("--close", None) => {
                let given = arguments.get(at..at + 2).ok_or("`--close` needs a port and a time")?;
                at += 2;
                args.feeds.push(Feed { port: number("--close", given.first())? as usize, at_ms: number("--close", given.get(1))?, bytes: None });
            }
            _ => return Err(format!("unknown argument `{argument}`")),
        }
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    fn seed(line: &str) -> Result<Option<u64>, String> {
        parse(&words(line)).map(|args| args.seed)
    }

    #[test]
    fn args_the_command_line_takes_a_seed() {
        assert_eq!(seed(""), Ok(None));
        assert_eq!(seed("--seed 7"), Ok(Some(7)));
        assert_eq!(seed("--seed=9"), Ok(Some(9)));
        assert_eq!(seed("--seed"), Err("`--seed` needs a value".to_string()));
        assert_eq!(seed("--seed x"), Err("`--seed` is a number, not x".to_string()));
        assert_eq!(seed("--seed 1 --seed=2"), Err("`--seed` is given twice".to_string()));
        assert_eq!(seed("--fast"), Err("unknown argument `--fast`".to_string()));
    }

    #[test]
    fn args_the_command_line_leaves_what_follows_two_dashes_to_the_program() {
        let args = parse(&words("--seed 3 -- a --seed b")).unwrap();
        assert_eq!((args.seed, args.program), (Some(3), words("a --seed b")));
        assert_eq!(parse(&words("--")).unwrap().program, Vec::<String>::new());
        assert_eq!(parse(&words("a")), Err("unknown argument `a`".to_string()));
    }

    #[test]
    fn args_the_command_line_takes_the_pushes_to_ports() {
        let args = parse(&words("--feed 0 5 1,2,255 --close 0 9 --seed 3")).unwrap();
        assert_eq!(args.seed, Some(3));
        assert_eq!(args.feeds, [Feed { port: 0, at_ms: 5, bytes: Some(vec![1, 2, 255]) }, Feed { port: 0, at_ms: 9, bytes: None }]);
        assert_eq!(parse(&words("--feed 0 5")), Err("`--feed` needs a port, a time and bytes".to_string()));
        assert_eq!(parse(&words("--feed 0 5 256")), Err("`--feed` bytes are numbers up to 255, not 256".to_string()));
        assert_eq!(parse(&words("--close 0")), Err("`--close` needs a port and a time".to_string()));
    }
}
